use std::io::Read;

use anyhow::{Context, anyhow};
use flate2::read::GzDecoder;
use md5::{Digest, Md5};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::crypto::{AesMode, CipherOutputFormat, decrypt_aes, encrypt_aes, encrypt_rsa};

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
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| BASE62[rng.gen_range(0..=61)] as char)
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

pub fn decrypt_eapi_response(
    encrypted_params: &str,
    aeapi: bool,
) -> Result<Map<String, Value>, String> {
    let decrypted = decrypt_aes(
        encrypted_params,
        AesMode::Ecb,
        EAPI_KEY,
        "",
        CipherOutputFormat::Hex,
    )?;

    let text = if aeapi {
        gunzip_to_string(&decrypted).map_err(|err| err.to_string())?
    } else {
        String::from_utf8(decrypted).map_err(|err| err.to_string())?
    };

    parse_json_record(&text).map_err(|err| err.to_string())
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
