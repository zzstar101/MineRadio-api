#![allow(dead_code)]
// These crypto helpers are intentionally kept as forward-compatible utilities and
// will be enabled as more sidecar features migrate to Rust.

use std::io::Read;

use anyhow::{Context, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use flate2::read::GzDecoder;
use md5::{Digest, Md5};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::common::{AesMode, CipherOutputFormat, decrypt_aes, encrypt_aes, encrypt_rsa};

const ID_XOR_KEY_1: &[u8] = b"3go8&$8*3*3h0k(2)2";
const K0: &[u8] = b"0123456789ABCDEF";
const IV: &str = "0102030405060708";
const PRESET_KEY: &str = "0CoJUm6Qyw8W8jud";
const LINUXAPI_KEY: &str = "rFgB&h#%2?^eDg:Q";
const EAPI_KEY: &str = "e82ckenh8dichen8";
const BASE62: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const WEAPI_PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\n\
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDgtQn2JZ34ZC28NWYpAUd98iZ37BUrX/aKzmFbt7clFSs6sXqHauqKWqdtLkF2KexO40H1YTX8z2lSgBBOAxLsvaklV8k4cBFK9snQXE9/DDaFt6Rr7iVZMldczhC0JNgTz+SHXT6CBHuX3e9SdB1Ua44oncaTWz7OBGLbCiK45wIDAQAB\n\
-----END PUBLIC KEY-----";
const EAPI_DELIMITER: &str = "-36cd479b6b5-";

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

pub fn encrypt_weapi_rsa(plaintext: &str, public_key: Option<&str>) -> Result<String, String> {
    encrypt_rsa(plaintext, public_key.unwrap_or(WEAPI_PUBLIC_KEY))
}

pub fn generate_weapi_secret_key() -> String {
    let mut rng = rand::rng();

    (0..16)
        .map(|_| BASE62[rng.random_range(0..=61)] as char)
        .collect()
}

pub fn encrypt_weapi(object: &Value, secret_key: Option<&str>) -> Result<WeapiParams, String> {
    let secret_key = secret_key
        .map(str::to_owned)
        .unwrap_or_else(generate_weapi_secret_key);
    let text = serde_json::to_string(object)
        .context("serialize weapi payload")
        .map_err(|err| err.to_string())?;
    let reversed_secret_key: String = secret_key.chars().rev().collect();

    Ok(WeapiParams {
        enc_sec_key: encrypt_weapi_rsa(&reversed_secret_key, None)?,
        params: encrypt_aes(
            &encrypt_aes(
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

pub fn encrypt_linuxapi(object: &Value) -> Result<LinuxapiParams, String> {
    let text = serde_json::to_string(object)
        .context("serialize linuxapi payload")
        .map_err(|err| err.to_string())?;
    Ok(LinuxapiParams {
        eparams: encrypt_aes(
            &text,
            AesMode::Ecb,
            LINUXAPI_KEY,
            "",
            CipherOutputFormat::Hex,
        )?,
    })
}

pub fn encrypt_eapi(url: &str, object: EapiBody<'_>) -> Result<EapiParams, String> {
    let text = match object {
        EapiBody::Json(value) => serde_json::to_string(value)
            .context("serialize eapi payload")
            .map_err(|err| err.to_string())?,
        EapiBody::Text(text) => text.to_owned(),
    };
    let message = format!("nobody{url}use{text}md5forencrypt");
    let digest = format!("{:x}", Md5::digest(message.as_bytes()));
    let data = format!("{url}{EAPI_DELIMITER}{text}{EAPI_DELIMITER}{digest}");

    Ok(EapiParams {
        params: encrypt_aes(&data, AesMode::Ecb, EAPI_KEY, "", CipherOutputFormat::Hex)?,
    })
}

pub fn decrypt_eapi_response(encrypted_params: &[u8], aeapi: bool) -> Result<Vec<u8>, String> {
    let encrypted_hex = hex::encode_upper(encrypted_params);
    let decrypted = decrypt_aes(
        &encrypted_hex,
        AesMode::Ecb,
        EAPI_KEY,
        "",
        CipherOutputFormat::Hex,
    )?;

    if aeapi {
        gunzip_to_bytes(&decrypted).map_err(|err| err.to_string())
    } else {
        Ok(decrypted)
    }
}

pub fn decrypt_eapi_request(encrypted_params: &str) -> Result<Option<EapiReqDecrypted>, String> {
    let decrypted = decrypt_eapi(encrypted_params)?;
    let Some((url, rest)) = decrypted.split_once(EAPI_DELIMITER) else {
        return Ok(None);
    };
    let Some((data, _digest)) = rest.split_once(EAPI_DELIMITER) else {
        return Ok(None);
    };

    Ok(Some(EapiReqDecrypted {
        data: parse_json_record(data).map_err(|err| err.to_string())?,
        url: url.to_owned(),
    }))
}

pub fn decrypt_eapi(cipher: &str) -> Result<String, String> {
    let decrypted = decrypt_aes(cipher, AesMode::Ecb, EAPI_KEY, "", CipherOutputFormat::Hex)?;
    String::from_utf8(decrypted)
        .context("eapi decrypted payload is not utf-8")
        .map_err(|err| err.to_string())
}

fn gunzip_to_bytes(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(bytes);
    let mut output = Vec::new();
    // TODO: Add a decompressed-size limit here after we confirm the real EAPI payload size range.
    decoder
        .read_to_end(&mut output)
        .context("gunzip eapi response")?;
    Ok(output)
}

fn parse_json_record(text: &str) -> anyhow::Result<Map<String, Value>> {
    match serde_json::from_str::<Value>(text).context("parse json object")? {
        Value::Object(object) => Ok(object),
        _ => Err(anyhow!("Expected JSON object payload")),
    }
}

pub fn generate_deviceid() -> String {
    let mut a = rand::rng();
    (0..52)
        .map(|_| {
            let b = a.random_range(0..K0.len());
            K0[b] as char
        })
        .collect()
}

pub fn cloudmusic_dll_encode_id(some_id: &str) -> String {
    let input = some_id.as_bytes();

    let xored: Vec<u8> = input
        .iter()
        .enumerate()
        .map(|(i, &byte)| byte ^ ID_XOR_KEY_1[i % ID_XOR_KEY_1.len()])
        .collect();

    let digest = Md5::digest(&xored);

    BASE64.encode(digest)
}

use sha2::Sha256;

use mac_address::get_mac_address;

pub fn get_real_mac_address() -> Option<String> {
    let mac = get_mac_address().ok()??;

    Some(mac.to_string().to_uppercase())
}

pub fn generate_random_mac() -> String {
    let mut rng = rand::rng();

    let mut bytes = [0u8; 6];
    rng.fill(&mut bytes);

    // 保证是单播地址
    bytes[0] &= 0xfe;

    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
    )
}

pub fn generate_wnmcid() -> String {
    const CHARACTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz";

    let mut rng = rand::rng();

    let random_string: String = (0..6)
        .map(|_| {
            let index = rng.random_range(0..CHARACTERS.len());
            CHARACTERS[index] as char
        })
        .collect();

    format!(
        "{}.{}.01.0",
        random_string,
        chrono::Utc::now().timestamp_millis()
    )
}

pub fn generate_ntes_nuid() -> String {
    let mut rng = rand::rng();
    let mut bytes = [0u8; 32];

    rng.fill(&mut bytes);

    hex::encode(bytes)
}

pub fn generate_client_sign(device_id: &str, secret_key: &str) -> String {
    let hex_device_id = hex::encode(device_id.as_bytes());

    let sign_string = format!(
        "{}@@@{}",
        generate_random_mac(),
        &hex_device_id[..hex_device_id.len().min(40)]
    );

    let mut hasher = Sha256::new();
    hasher.update(sign_string.as_bytes());
    hasher.update(secret_key.as_bytes());

    let hash = hex::encode(hasher.finalize());

    format!("{}@@@@@@{}", sign_string, hash)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn eapi_request_decrypts_generated_params() {
        let body = json!({ "id": 123, "csrf_token": "" });
        let encrypted = encrypt_eapi("/api/song/detail", EapiBody::Json(&body)).unwrap();
        assert_eq!(
            encrypted.params,
            "7D398AA5036D61F11B22021C618C242421D51F26B6A0246E121BFC7B69A3481F1B9A150C4A39113850F18DC62989A66D644F8B358D237F37959FBD383C9E0FF246B0E364C81E80A53B281B1A8E79FF4D4BD4FDFDDD0FAB97B9BA28E33602FCD4CFBFCE1DC1C4F4737873E98E44F5D059"
        );
        let decrypted = decrypt_eapi_request(&encrypted.params).unwrap().unwrap();

        assert_eq!(decrypted.url, "/api/song/detail");
        assert_eq!(decrypted.data.get("id"), Some(&json!(123)));
        assert_eq!(decrypted.data.get("csrf_token"), Some(&json!("")));
    }

    #[test]
    fn linuxapi_encrypts_as_hex_and_decrypts_as_json() {
        let body = json!({ "method": "POST", "url": "/api/test" });
        let encrypted = encrypt_linuxapi(&body).unwrap();
        let decrypted = decrypt_aes(
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
        let encrypted = encrypt_weapi(&body, Some("abcdefghijklmnop")).unwrap();

        assert_eq!(encrypted.enc_sec_key.len(), 256);
        assert_eq!(
            encrypted.enc_sec_key,
            "d15a1683c992095d0c234c19966605c5c5964911268bbeda8cb8d08d834913e59d53b32358903a121b5fca784c1f5ae44951fd02524df58ecc98e52cc7cf8689b42c2e93ddf05b0592512d87f5960467e2f086c018849d76014d323500e30f13ef4cafbb0cf5a66731a3f1776c75ca35d0062dac70a3e33245afabcf47938487"
        );
        assert_eq!(
            encrypted.params,
            "gHkCij6ElKidi+zv9289kG2vFoO4JAR+6FJdSQK16AmoQDz/ZbaRxBk5QyRuBYZr"
        );
        assert_eq!(
            encrypted,
            encrypt_weapi(&body, Some("abcdefghijklmnop")).unwrap()
        );
    }
}
