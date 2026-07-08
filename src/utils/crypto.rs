use std::io::Read;

use aes::Aes128;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit, block_padding::Pkcs7};
use anyhow::{Context, anyhow};
use base64::{Engine as _, engine::general_purpose};
use flate2::read::GzDecoder;
use md5::{Digest, Md5};
use rand::Rng;
use rsa::{BigUint, RsaPublicKey, pkcs8::DecodePublicKey, traits::PublicKeyParts};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const IV: &str = "0102030405060708";
const PRESET_KEY: &str = "0CoJUm6Qyw8W8jud";
const LINUXAPI_KEY: &str = "rFgB&h#%2?^eDg:Q";
const EAPI_KEY: &str = "e82ckenh8dichen8";
const BASE62: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const WEAPI_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\n\
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDgtQn2JZ34ZC28NWYpAUd98iZ37BUrX/aKzmFbt7clFSs6sXqHauqKWqdtLkF2KexO40H1YTX8z2lSgBBOAxLsvaklV8k4cBFK9snQXE9/DDaFt6Rr7iVZMldczhC0JNgTz+SHXT6CBHuX3e9SdB1Ua44oncaTWz7OBGLbCiK45wIDAQAB\n\
-----END PUBLIC KEY-----";
const EAPI_DELIMITER: &str = "-36cd479b6b5-";
const RSA_BLOCK_SIZE: usize = 128;

type Aes128CbcEnc = cbc::Encryptor<Aes128>;
type Aes128CbcDec = cbc::Decryptor<Aes128>;
type Aes128EcbEnc = ecb::Encryptor<Aes128>;
type Aes128EcbDec = ecb::Decryptor<Aes128>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AesMode {
    Cbc,
    Ecb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CipherOutputFormat {
    Base64,
    Hex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeapiParams {
    #[serde(rename = "encSecKey")]
    pub enc_sec_key: String,
    pub params: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxapiParams {
    pub eparams: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EapiParams {
    pub params: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EapiReqDecrypted {
    pub data: Map<String, Value>,
    pub url: String,
}

#[derive(Debug, Clone, Copy)]
pub enum EapiBody<'a> {
    Json(&'a Value),
    Text(&'a str),
}

pub fn aes_encrypt(
    text: &str,
    mode: AesMode,
    key: &str,
    iv: &str,
    format: CipherOutputFormat,
) -> anyhow::Result<String> {
    let encrypted = match mode {
        AesMode::Cbc => encrypt_cbc(text.as_bytes(), key.as_bytes(), iv.as_bytes())?,
        AesMode::Ecb => encrypt_ecb(text.as_bytes(), key.as_bytes())?,
    };

    Ok(match format {
        CipherOutputFormat::Hex => to_hex_upper(&encrypted),
        CipherOutputFormat::Base64 => general_purpose::STANDARD.encode(encrypted),
    })
}

pub fn aes_decrypt(
    ciphertext: &str,
    mode: AesMode,
    key: &str,
    iv: &str,
    format: CipherOutputFormat,
) -> anyhow::Result<Vec<u8>> {
    let encrypted = match format {
        CipherOutputFormat::Hex => from_hex(ciphertext)?,
        CipherOutputFormat::Base64 => general_purpose::STANDARD
            .decode(ciphertext)
            .context("invalid base64 ciphertext")?,
    };

    match mode {
        AesMode::Cbc => decrypt_cbc(&encrypted, key.as_bytes(), iv.as_bytes()),
        AesMode::Ecb => decrypt_ecb(&encrypted, key.as_bytes()),
    }
}

pub fn rsa_encrypt(plaintext: &str, public_key: Option<&str>) -> anyhow::Result<String> {
    let public_key = parse_rsa_public_key(public_key.unwrap_or(WEAPI_PUBLIC_KEY))?;
    let mut padded = [0u8; RSA_BLOCK_SIZE];
    let bytes = plaintext.as_bytes();
    if bytes.len() > RSA_BLOCK_SIZE {
        return Err(anyhow!("rsa plaintext is longer than block size"));
    }

    padded[RSA_BLOCK_SIZE - bytes.len()..].copy_from_slice(bytes);
    let message = BigUint::from_bytes_be(&padded);
    let encrypted = message.modpow(public_key.e(), public_key.n());
    let mut encrypted_bytes = encrypted.to_bytes_be();
    let key_size = public_key.size();
    if encrypted_bytes.len() > key_size {
        return Err(anyhow!("rsa encrypted block is longer than key size"));
    }

    let mut output = vec![0u8; key_size];
    output[key_size - encrypted_bytes.len()..].copy_from_slice(&encrypted_bytes);
    encrypted_bytes.fill(0);
    Ok(to_hex_lower(&output))
}

fn parse_rsa_public_key(public_key: &str) -> anyhow::Result<RsaPublicKey> {
    if let Ok(parsed) = RsaPublicKey::from_public_key_pem(public_key) {
        return Ok(parsed);
    }

    let base64_body: String = public_key
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let der = general_purpose::STANDARD
        .decode(base64_body)
        .context("invalid rsa public key")?;
    RsaPublicKey::from_public_key_der(&der).context("invalid rsa public key")
}

pub fn create_weapi_secret_key() -> String {
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| BASE62[rng.gen_range(0..=61)] as char)
        .collect()
}

pub fn weapi(object: &Value, secret_key: Option<&str>) -> anyhow::Result<WeapiParams> {
    let secret_key = secret_key
        .map(str::to_owned)
        .unwrap_or_else(create_weapi_secret_key);
    let text = serde_json::to_string(object).context("serialize weapi payload")?;
    let reversed_secret_key: String = secret_key.chars().rev().collect();

    Ok(WeapiParams {
        enc_sec_key: rsa_encrypt(&reversed_secret_key, None)?,
        params: aes_encrypt(
            &aes_encrypt(
                &text,
                AesMode::Cbc,
                PRESET_KEY,
                IV,
                CipherOutputFormat::Base64,
            )?,
            AesMode::Cbc,
            &secret_key,
            IV,
            CipherOutputFormat::Base64,
        )?,
    })
}

pub fn linuxapi(object: &Value) -> anyhow::Result<LinuxapiParams> {
    let text = serde_json::to_string(object).context("serialize linuxapi payload")?;
    Ok(LinuxapiParams {
        eparams: aes_encrypt(
            &text,
            AesMode::Ecb,
            LINUXAPI_KEY,
            "",
            CipherOutputFormat::Hex,
        )?,
    })
}

pub fn eapi(url: &str, object: EapiBody<'_>) -> anyhow::Result<EapiParams> {
    let text = match object {
        EapiBody::Json(value) => serde_json::to_string(value).context("serialize eapi payload")?,
        EapiBody::Text(text) => text.to_owned(),
    };
    let message = format!("nobody{url}use{text}md5forencrypt");
    let digest = format!("{:x}", Md5::digest(message.as_bytes()));
    let data = format!("{url}{EAPI_DELIMITER}{text}{EAPI_DELIMITER}{digest}");

    Ok(EapiParams {
        params: aes_encrypt(&data, AesMode::Ecb, EAPI_KEY, "", CipherOutputFormat::Hex)?,
    })
}

pub fn eapi_res_decrypt(encrypted_params: &str, aeapi: bool) -> Option<Map<String, Value>> {
    let decrypted = aes_decrypt(
        encrypted_params,
        AesMode::Ecb,
        EAPI_KEY,
        "",
        CipherOutputFormat::Hex,
    )
    .ok()?;

    let text = if aeapi {
        gunzip_to_string(&decrypted).ok()?
    } else {
        String::from_utf8(decrypted).ok()?
    };

    parse_json_record(&text).ok()
}

pub fn eapi_req_decrypt(encrypted_params: &str) -> anyhow::Result<Option<EapiReqDecrypted>> {
    let decrypted = decrypt(encrypted_params)?;
    let Some((url, rest)) = decrypted.split_once(EAPI_DELIMITER) else {
        return Ok(None);
    };
    let Some((data, _digest)) = rest.split_once(EAPI_DELIMITER) else {
        return Ok(None);
    };

    Ok(Some(EapiReqDecrypted {
        data: parse_json_record(data)?,
        url: url.to_owned(),
    }))
}

pub fn decrypt(cipher: &str) -> anyhow::Result<String> {
    let decrypted = aes_decrypt(cipher, AesMode::Ecb, EAPI_KEY, "", CipherOutputFormat::Hex)?;
    String::from_utf8(decrypted).context("eapi decrypted payload is not utf-8")
}

fn encrypt_cbc(plaintext: &[u8], key: &[u8], iv: &[u8]) -> anyhow::Result<Vec<u8>> {
    ensure_aes_key(key)?;
    ensure_aes_iv(iv)?;
    let mut output = vec![0u8; plaintext.len() + 16];
    let encrypted = Aes128CbcEnc::new(key.into(), iv.into())
        .encrypt_padded_b2b_mut::<Pkcs7>(plaintext, &mut output)
        .map_err(|err| anyhow!("aes cbc encrypt failed: {err}"))?;
    Ok(encrypted.to_vec())
}

fn decrypt_cbc(ciphertext: &[u8], key: &[u8], iv: &[u8]) -> anyhow::Result<Vec<u8>> {
    ensure_aes_key(key)?;
    ensure_aes_iv(iv)?;
    let mut output = vec![0u8; ciphertext.len()];
    let decrypted = Aes128CbcDec::new(key.into(), iv.into())
        .decrypt_padded_b2b_mut::<Pkcs7>(ciphertext, &mut output)
        .map_err(|err| anyhow!("aes cbc decrypt failed: {err}"))?;
    Ok(decrypted.to_vec())
}

fn encrypt_ecb(plaintext: &[u8], key: &[u8]) -> anyhow::Result<Vec<u8>> {
    ensure_aes_key(key)?;
    let mut output = vec![0u8; plaintext.len() + 16];
    let encrypted = Aes128EcbEnc::new(key.into())
        .encrypt_padded_b2b_mut::<Pkcs7>(plaintext, &mut output)
        .map_err(|err| anyhow!("aes ecb encrypt failed: {err}"))?;
    Ok(encrypted.to_vec())
}

fn decrypt_ecb(ciphertext: &[u8], key: &[u8]) -> anyhow::Result<Vec<u8>> {
    ensure_aes_key(key)?;
    let mut output = vec![0u8; ciphertext.len()];
    let decrypted = Aes128EcbDec::new(key.into())
        .decrypt_padded_b2b_mut::<Pkcs7>(ciphertext, &mut output)
        .map_err(|err| anyhow!("aes ecb decrypt failed: {err}"))?;
    Ok(decrypted.to_vec())
}

fn ensure_aes_key(key: &[u8]) -> anyhow::Result<()> {
    if key.len() == 16 {
        Ok(())
    } else {
        Err(anyhow!("aes-128 key must be 16 bytes"))
    }
}

fn ensure_aes_iv(iv: &[u8]) -> anyhow::Result<()> {
    if iv.len() == 16 {
        Ok(())
    } else {
        Err(anyhow!("aes cbc iv must be 16 bytes"))
    }
}

fn gunzip_to_string(bytes: &[u8]) -> anyhow::Result<String> {
    let mut decoder = GzDecoder::new(bytes);
    let mut output = String::new();
    decoder
        .read_to_string(&mut output)
        .context("gunzip eapi response")?;
    Ok(output)
}

fn parse_json_record(text: &str) -> anyhow::Result<Map<String, Value>> {
    match serde_json::from_str::<Value>(text).context("parse json object")? {
        Value::Object(object) => Ok(object),
        _ => Err(anyhow!("Expected JSON object payload")),
    }
}

fn to_hex_upper(bytes: &[u8]) -> String {
    encode_hex(bytes, b"0123456789ABCDEF")
}

fn to_hex_lower(bytes: &[u8]) -> String {
    encode_hex(bytes, b"0123456789abcdef")
}

fn encode_hex(bytes: &[u8], alphabet: &[u8; 16]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(alphabet[(byte >> 4) as usize] as char);
        output.push(alphabet[(byte & 0x0f) as usize] as char);
    }
    output
}

fn from_hex(text: &str) -> anyhow::Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return Err(anyhow!("hex ciphertext length must be even"));
    }

    let mut output = Vec::with_capacity(text.len() / 2);
    for chunk in text.as_bytes().chunks_exact(2) {
        let high = hex_value(chunk[0])?;
        let low = hex_value(chunk[1])?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn hex_value(byte: u8) -> anyhow::Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(anyhow!("invalid hex character")),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn aes_cbc_round_trips_base64() {
        let encrypted = aes_encrypt(
            r#"{"hello":"world"}"#,
            AesMode::Cbc,
            PRESET_KEY,
            IV,
            CipherOutputFormat::Base64,
        )
        .unwrap();
        let decrypted = aes_decrypt(
            &encrypted,
            AesMode::Cbc,
            PRESET_KEY,
            IV,
            CipherOutputFormat::Base64,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(decrypted).unwrap(),
            r#"{"hello":"world"}"#
        );
    }

    #[test]
    fn eapi_request_decrypts_generated_params() {
        let body = json!({ "id": 123, "csrf_token": "" });
        let encrypted = eapi("/api/song/detail", EapiBody::Json(&body)).unwrap();
        assert_eq!(
            encrypted.params,
            "7D398AA5036D61F11B22021C618C242421D51F26B6A0246E121BFC7B69A3481F1B9A150C4A39113850F18DC62989A66D644F8B358D237F37959FBD383C9E0FF246B0E364C81E80A53B281B1A8E79FF4D4BD4FDFDDD0FAB97B9BA28E33602FCD4CFBFCE1DC1C4F4737873E98E44F5D059"
        );
        let decrypted = eapi_req_decrypt(&encrypted.params).unwrap().unwrap();

        assert_eq!(decrypted.url, "/api/song/detail");
        assert_eq!(decrypted.data.get("id"), Some(&json!(123)));
        assert_eq!(decrypted.data.get("csrf_token"), Some(&json!("")));
    }

    #[test]
    fn linuxapi_encrypts_as_hex_and_decrypts_as_json() {
        let body = json!({ "method": "POST", "url": "/api/test" });
        let encrypted = linuxapi(&body).unwrap();
        let decrypted = aes_decrypt(
            &encrypted.eparams,
            AesMode::Ecb,
            LINUXAPI_KEY,
            "",
            CipherOutputFormat::Hex,
        )
        .unwrap();

        assert_eq!(
            encrypted.eparams,
            "A0D9583F4C5FF68DE851D2893A49DE988005EE33CD858A86B534CA8C49710E941B3C2A35B43461435FFC433F63AC1194"
        );
        assert!(
            encrypted
                .eparams
                .chars()
                .all(|char| !char.is_ascii_lowercase())
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&decrypted).unwrap(),
            json!({ "method": "POST", "url": "/api/test" })
        );
    }

    #[test]
    fn weapi_uses_fixed_secret_key_deterministically() {
        let body = json!({ "s": "name", "type": 1 });
        let encrypted = weapi(&body, Some("abcdefghijklmnop")).unwrap();

        assert_eq!(encrypted.enc_sec_key.len(), 256);
        assert_eq!(
            encrypted.enc_sec_key,
            "d15a1683c992095d0c234c19966605c5c5964911268bbeda8cb8d08d834913e59d53b32358903a121b5fca784c1f5ae44951fd02524df58ecc98e52cc7cf8689b42c2e93ddf05b0592512d87f5960467e2f086c018849d76014d323500e30f13ef4cafbb0cf5a66731a3f1776c75ca35d0062dac70a3e33245afabcf47938487"
        );
        assert_eq!(
            encrypted.params,
            "gHkCij6ElKidi+zv9289kG2vFoO4JAR+6FJdSQK16AmoQDz/ZbaRxBk5QyRuBYZr"
        );
        assert_eq!(encrypted, weapi(&body, Some("abcdefghijklmnop")).unwrap());
    }
}
