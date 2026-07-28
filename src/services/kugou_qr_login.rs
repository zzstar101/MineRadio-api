use aes::Aes256;
use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use md5::{Digest, Md5};
use rand::RngExt;
use reqwest::header::{HeaderMap, SET_COOKIE};
use rsa::{BigUint, RsaPublicKey, traits::PublicKeyParts};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{
    services::auth_session::set_runtime_provider_cookie,
    types::{ProviderId, ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
    utils::cryptors::{
        decrypt_kugou_register_payload, encrypt_kugou_register_payload, encrypt_kugou_register_rsa,
    },
};

#[derive(Clone, Debug)]
pub struct KugouQrCode {
    pub key: String,
    pub image: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct KugouQrPollResult {
    pub code: i64,
    pub message: Option<String>,
    pub logged_in: bool,
    pub scanned: Option<bool>,
    pub expired: Option<bool>,
    pub cookie: Option<String>,
    pub token: Option<String>,
    pub user_id: Option<String>,
}

#[async_trait]
pub trait KugouQrLoginApi: Send + Sync {
    async fn create_qr(&self) -> anyhow::Result<KugouQrCode>;
    async fn check_qr(&self, key: &str) -> anyhow::Result<KugouQrPollResult>;
}

pub struct KugouQrLoginDeps {
    pub api: Box<dyn KugouQrLoginApi>,
}

impl Default for KugouQrLoginDeps {
    fn default() -> Self {
        Self {
            api: Box::new(KugouQrHttpApi::default()),
        }
    }
}

pub struct KugouQrLoginService {
    deps: KugouQrLoginDeps,
    image_cache: tokio::sync::Mutex<HashMap<String, ProviderLoginQrImage>>,
}

impl KugouQrLoginService {
    pub async fn create_key(&self) -> anyhow::Result<ProviderLoginQrKey> {
        let qr = self.deps.api.create_qr().await?;
        let key = required_value(&qr.key, "KUGOU_QR_KEY_MISSING")?;
        let image = required_value(&qr.image, "KUGOU_QR_IMAGE_MISSING")?;
        let payload = ProviderLoginQrImage {
            provider: ProviderId::Kugou,
            key: key.clone(),
            img: image,
            url: qr.url.filter(|url| !url.trim().is_empty()),
        };
        self.image_cache.lock().await.insert(key.clone(), payload);
        Ok(ProviderLoginQrKey {
            provider: ProviderId::Kugou,
            key,
        })
    }

    pub async fn create_image(&self, key: &str) -> anyhow::Result<ProviderLoginQrImage> {
        let key = required_value(key, "KUGOU_QR_KEY_REQUIRED")?;
        self.image_cache
            .lock()
            .await
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_IMAGE_MISSING"))
    }

    pub async fn check(&self, key: &str) -> anyhow::Result<ProviderLoginQrCheck> {
        let key = required_value(key, "KUGOU_QR_KEY_REQUIRED")?;
        let result = self.deps.api.check_qr(&key).await?;
        let mut stored = false;

        if result.logged_in {
            let cookie = result
                .cookie
                .as_deref()
                .map(str::trim)
                .filter(|cookie| !cookie.is_empty())
                .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_COOKIE_MISSING"))?;
            set_runtime_provider_cookie(ProviderId::Kugou, cookie.to_owned())
                .await
                .map_err(|err| anyhow::anyhow!(err))?;
            stored = true;
            self.image_cache.lock().await.remove(&key);
        }

        Ok(ProviderLoginQrCheck {
            provider: ProviderId::Kugou,
            key,
            code: result.code,
            message: result.message,
            logged_in: result.logged_in,
            scanned: result.scanned,
            expired: result.expired,
            stored: Some(stored),
        })
    }
}

pub fn create_kugou_qr_login_service(deps: KugouQrLoginDeps) -> KugouQrLoginService {
    KugouQrLoginService {
        deps,
        image_cache: tokio::sync::Mutex::new(HashMap::new()),
    }
}

#[derive(Clone, Default)]
pub struct KugouQrHttpApi {
    client: reqwest::Client,
    devices: Arc<tokio::sync::Mutex<HashMap<String, KugouQrDevice>>>,
}

const KUGOU_QR_URL: &str = "https://login-user.kugou.com/v2/qrcode";
const KUGOU_QR_CHECK_URL: &str = "https://login-user.kugou.com/v2/get_userinfo_qrcode";
const KUGOU_TOKEN_LOGIN_URL: &str = "https://loginservice.kugou.com/v1/login_by_token_get";
const KUGOU_REGISTER_DEVICE_URL: &str = "https://userservice.kugou.com/risk/v2/r_register_dev";
const KUGOU_QR_PAGE: &str = "https://h5.kugou.com/apps/loginQRCode/html/index.html";
const KUGOU_QR_TEXT: &str = "https://h5.kugou.com/apps/loginQRCode/html/index.html?appid=1014&";
const KUGOU_QR_REFERER: &str = "https://login-user.kugou.com/login/?appid=1014&ref=https://www.kugou.com/reg/web/&redirect_uri=https://staticssl.kugou.com/common/html/login/regok.html&callback=UsLoginCallback";
const KUGOU_QR_WEB_SALT: &str = "NVPh5oo715z5DIWAeQlhMDsWXXQV4hwt";
const KUGOU_REGISTER_SIGNATURE_SALT: &str = "OIlwieks28dk2k092lksi2UIkp";
const KUGOU_RSA_MODULUS_HEX: &str = "B1B1EC76A1BBDBF0D18E8CD9A87E53FA3881E2F004C67C9DDA2CA677DBEFA3D61DF8463FE12D84FF4B4699E02C9D41CAB917F5A8FB9E35580C4BDF97763A0420A476295D763EE10174E6F9EBF7DF8A77BA5B20CDA4EE705DEF5BBA3C88567B9656E52C9CD5CD95CA735FF2D25F762B133273EEEB7B4F3EA8B6DA29040F3B67CD";
const KUGOU_AES_KEY_CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const KUGOU_REGISTER_KEY_CHARS: &[u8] = b"1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ";

type Aes256CbcEnc = cbc::Encryptor<Aes256>;

#[async_trait]
impl KugouQrLoginApi for KugouQrHttpApi {
    async fn create_qr(&self) -> anyhow::Result<KugouQrCode> {
        let (mid, uuid) = {
            let uuid = random_kugou_uuid();
            (md5_hex(uuid.as_bytes()), uuid)
        };
        let dfid = self.register_device(&mid, &uuid).await?;

        let primary_params = build_qr_params(current_time_millis(), "8131", &mid, &dfid, &mid);
        let primary = self.request_qr(primary_params.clone()).await;
        match primary {
            Ok(qr) => {
                self.remember_device(&qr.key, &primary_params).await;
                Ok(qr)
            }
            Err(primary_error) => {
                let fallback_params =
                    build_qr_params(current_time_seconds(), "20489", &mid, &dfid, "-");
                match self.request_qr(fallback_params.clone()).await {
                    Ok(qr) => {
                        self.remember_device(&qr.key, &fallback_params).await;
                        Ok(qr)
                    }
                    Err(fallback_error) => Err(anyhow::anyhow!(
                        "KUGOU_QR_PRIMARY_FAILED: {primary_error}; KUGOU_QR_FALLBACK_FAILED: {fallback_error}"
                    )),
                }
            }
        }
    }

    async fn check_qr(&self, key: &str) -> anyhow::Result<KugouQrPollResult> {
        let key = required_value(key, "KUGOU_QR_KEY_REQUIRED")?;
        let device = self
            .devices
            .lock()
            .await
            .get(&key)
            .cloned()
            .unwrap_or_else(|| KugouQrDevice {
                mid: md5_hex(b"mineradio:"),
                dfid: "-".to_owned(),
                uuid: md5_hex(b"mineradio:"),
            });
        let params = build_check_params(current_time_millis(), &device, &key);
        let response = self
            .client
            .get(KUGOU_QR_CHECK_URL)
            .query(&params)
            .header("user-agent", "Mozilla/5.0")
            .header("referer", KUGOU_QR_REFERER)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("KUGOU_QR_CHECK_HTTP_{}", response.status().as_u16());
        }
        let mut result = parse_qr_poll_response(response.json::<Value>().await?)?;
        if result.logged_in {
            let token = result
                .token
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_TOKEN_MISSING"))?;
            let user_id = result
                .user_id
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_USER_ID_MISSING"))?;
            result.cookie = Some(self.exchange_token(&device, token, user_id).await?);
        }
        if result.expired == Some(true) || result.logged_in {
            self.devices.lock().await.remove(&key);
        }
        Ok(result)
    }
}

impl KugouQrHttpApi {
    async fn remember_device(&self, key: &str, params: &BTreeMap<String, String>) {
        self.devices.lock().await.insert(
            key.to_owned(),
            KugouQrDevice {
                mid: params.get("mid").cloned().unwrap_or_default(),
                dfid: params.get("dfid").cloned().unwrap_or_default(),
                uuid: params.get("uuid").cloned().unwrap_or_default(),
            },
        );
    }

    async fn register_device(&self, mid: &str, uuid: &str) -> anyhow::Result<String> {
        let clienttime = current_time_seconds();
        let token = String::new();
        let user_id = "0".to_owned();
        let aes_key = random_kugou_register_key();
        let encrypted_body =
            encrypt_kugou_register_payload(&register_device_payload(uuid).to_string(), &aes_key)
                .map_err(|err| anyhow::anyhow!("KUGOU_REGISTER_AES_FAILED: {err}"))?;
        let uid = user_id
            .parse::<u64>()
            .map(Value::from)
            .unwrap_or(Value::from(0));
        let rsa_plaintext = json!({
            "aes": aes_key,
            "uid": uid,
            "token": token,
        })
        .to_string();
        let p = encrypt_kugou_register_rsa(&rsa_plaintext)
            .map_err(|err| anyhow::anyhow!("KUGOU_REGISTER_RSA_FAILED: {err}"))?;
        let mut params = BTreeMap::from([
            ("appid".to_owned(), "1005".to_owned()),
            ("clienttime".to_owned(), clienttime.to_string()),
            ("clientver".to_owned(), "20489".to_owned()),
            ("dfid".to_owned(), "-".to_owned()),
            ("mid".to_owned(), mid.to_owned()),
            ("p".to_owned(), p),
            ("part".to_owned(), "1".to_owned()),
            ("platid".to_owned(), "1".to_owned()),
            ("uuid".to_owned(), "-".to_owned()),
        ]);
        if !token.is_empty() {
            params.insert("token".to_owned(), token);
        }
        if !user_id.is_empty() {
            params.insert("userid".to_owned(), user_id);
        }
        params.insert(
            "signature".to_owned(),
            signature_kugou_android(&params, &encrypted_body),
        );

        let response = self
            .client
            .post(KUGOU_REGISTER_DEVICE_URL)
            .query(&params)
            .header(
                "user-agent",
                "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi",
            )
            .header("dfid", "-")
            .header("clienttime", clienttime.to_string())
            .header("mid", mid)
            .body(encrypted_body)
            .send()
            .await?;
        let http_status = response.status();
        if !http_status.is_success() {
            anyhow::bail!("KUGOU_REGISTER_HTTP_{}", http_status.as_u16());
        }
        let encrypted_response = BASE64.encode(response.bytes().await?);
        let body = decrypt_kugou_register_payload(&encrypted_response, &aes_key)
            .map_err(|err| anyhow::anyhow!("KUGOU_REGISTER_DECRYPT_FAILED: {err}"))?;
        let body = serde_json::from_slice::<Value>(&body)
            .map_err(|err| anyhow::anyhow!("KUGOU_REGISTER_RESPONSE_INVALID: {err}"))?;
        if body.get("status").and_then(Value::as_i64) != Some(1) {
            anyhow::bail!("KUGOU_REGISTER_FAILED");
        }
        body.get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("dfid"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|dfid| !dfid.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("KUGOU_REGISTER_DFID_MISSING"))
    }

    async fn request_qr(&self, params: BTreeMap<String, String>) -> anyhow::Result<KugouQrCode> {
        let response = self
            .client
            .get(KUGOU_QR_URL)
            .query(&params)
            .header("user-agent", "Mozilla/5.0")
            .header("referer", KUGOU_QR_REFERER)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("KUGOU_QR_HTTP_{}", response.status().as_u16());
        }
        parse_qr_response(response.json::<Value>().await?)
    }

    async fn exchange_token(
        &self,
        device: &KugouQrDevice,
        token: &str,
        user_id: &str,
    ) -> anyhow::Result<String> {
        let clienttime = current_time_seconds();
        let clienttime_ms = current_time_millis();
        let aes_key = random_aes_key();
        let params_value = encrypt_token_params(token, &aes_key)?;
        let pk_plaintext = serde_json::json!({
            "clienttime_ms": clienttime_ms,
            "key": aes_key,
        })
        .to_string();
        let pk = encrypt_kugou_rsa(&pk_plaintext)
            .map_err(|err| anyhow::anyhow!("KUGOU_TOKEN_RSA_FAILED: {err}"))?;
        let mut request_params = BTreeMap::from([
            ("appid".to_owned(), "1014".to_owned()),
            ("clientver".to_owned(), "1000".to_owned()),
            ("clienttime".to_owned(), clienttime.to_string()),
            ("mid".to_owned(), device.mid.clone()),
            ("uuid".to_owned(), device.uuid.clone()),
            ("dfid".to_owned(), device.dfid.clone()),
            ("dev".to_owned(), "web".to_owned()),
            ("userid".to_owned(), user_id.to_owned()),
            ("plat".to_owned(), "4".to_owned()),
            ("clienttime_ms".to_owned(), clienttime_ms.to_string()),
            ("pk".to_owned(), pk),
            ("params".to_owned(), params_value),
            ("srcappid".to_owned(), "2919".to_owned()),
        ]);
        let signature = signature_web_qr(&request_params);
        request_params.insert("signature".to_owned(), signature);
        let response = self
            .client
            .post(KUGOU_TOKEN_LOGIN_URL)
            .query(&request_params)
            .header("user-agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36 Edg/151.0.0.0")
            .header("referer", "https://login-user.kugou.com/")
            .header(
                "cookie",
                format!("kg_mid={}; kg_dfid={} ", device.mid, device.dfid),
            )
            .send()
            .await?;
        let http_status = response.status();
        let header_cookie = response_cookie_header(&response);
        let body = response.text().await?;
        if let Some(cookie) = header_cookie {
            return Ok(cookie);
        }
        if !http_status.is_success() {
            anyhow::bail!("KUGOU_TOKEN_LOGIN_HTTP_{}", http_status.as_u16());
        }
        if let Some(cookie) = cookie_from_response_body(&body) {
            return Ok(cookie);
        }
        let business_error = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| value.get("error_code").and_then(Value::as_i64))
            .map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        anyhow::bail!(
            "KUGOU_TOKEN_LOGIN_COOKIE_MISSING: HTTP_{} ERROR_CODE_{}",
            http_status.as_u16(),
            business_error
        )
    }
}

#[derive(Clone, Debug)]
struct KugouQrDevice {
    mid: String,
    dfid: String,
    uuid: String,
}

fn build_qr_params(
    clienttime: u128,
    clientver: &str,
    mid: &str,
    dfid: &str,
    uuid: &str,
) -> BTreeMap<String, String> {
    let mut params = BTreeMap::from([
        ("appid".to_owned(), "1014".to_owned()),
        ("clienttime".to_owned(), clienttime.to_string()),
        ("clientver".to_owned(), clientver.to_owned()),
        ("dfid".to_owned(), dfid.to_owned()),
        ("mid".to_owned(), mid.to_owned()),
        ("plat".to_owned(), "4".to_owned()),
        ("qrcode_txt".to_owned(), KUGOU_QR_TEXT.to_owned()),
        ("srcappid".to_owned(), "2919".to_owned()),
        ("type".to_owned(), "1".to_owned()),
        ("uuid".to_owned(), uuid.to_owned()),
    ]);
    let signature = signature_web_qr(&params);
    params.insert("signature".to_owned(), signature);
    params
}

fn build_check_params(
    clienttime: u128,
    device: &KugouQrDevice,
    qrcode: &str,
) -> BTreeMap<String, String> {
    let mut params = BTreeMap::from([
        ("appid".to_owned(), "1014".to_owned()),
        ("clienttime".to_owned(), clienttime.to_string()),
        ("clientver".to_owned(), "8131".to_owned()),
        ("dfid".to_owned(), device.dfid.clone()),
        ("mid".to_owned(), device.mid.clone()),
        ("plat".to_owned(), "4".to_owned()),
        ("qrcode".to_owned(), qrcode.to_owned()),
        ("srcappid".to_owned(), "2919".to_owned()),
        ("uuid".to_owned(), device.uuid.clone()),
    ]);
    let signature = signature_web_qr(&params);
    params.insert("signature".to_owned(), signature);
    params
}

fn signature_web_qr(params: &BTreeMap<String, String>) -> String {
    let pairs = params
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<String>();
    md5_hex_upper(format!("{KUGOU_QR_WEB_SALT}{pairs}{KUGOU_QR_WEB_SALT}").as_bytes())
}

fn signature_kugou_android(params: &BTreeMap<String, String>, data: &str) -> String {
    let pairs = params
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<String>();
    md5_hex(
        format!("{KUGOU_REGISTER_SIGNATURE_SALT}{pairs}{data}{KUGOU_REGISTER_SIGNATURE_SALT}")
            .as_bytes(),
    )
}

fn register_device_payload(uuid: &str) -> Value {
    json!({
        "availableRamSize": 4983533568u64,
        "availableRomSize": 48114719u64,
        "availableSDSize": 48114717u64,
        "basebandVer": "",
        "batteryLevel": 100,
        "batteryStatus": 3,
        "brand": "Redmi",
        "buildSerial": "unknown",
        "device": "marble",
        "imei": uuid,
        "imsi": "",
        "manufacturer": "Xiaomi",
        "uuid": uuid,
        "accelerometer": false,
        "accelerometerValue": "",
        "gravity": false,
        "gravityValue": "",
        "gyroscope": false,
        "gyroscopeValue": "",
        "light": false,
        "lightValue": "",
        "magnetic": false,
        "magneticValue": "",
        "orientation": false,
        "orientationValue": "",
        "pressure": false,
        "pressureValue": "",
        "step_counter": false,
        "step_counterValue": "",
        "temperature": false,
        "temperatureValue": ""
    })
}

fn parse_qr_response(body: Value) -> anyhow::Result<KugouQrCode> {
    let data = body
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_DATA_MISSING"))?;
    let key = data
        .get("qrcode")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_KEY_MISSING"))?;
    let image = data
        .get("qrcode_img")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_IMAGE_MISSING"))?;
    Ok(KugouQrCode {
        key: key.to_owned(),
        image: image.to_owned(),
        url: Some(format!("{KUGOU_QR_PAGE}?qrcode={key}")),
    })
}

fn parse_qr_poll_response(body: Value) -> anyhow::Result<KugouQrPollResult> {
    let data = body
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_CHECK_DATA_MISSING"))?;
    let status = data
        .get("status")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_CHECK_STATUS_MISSING"))?;
    let logged_in = status == 4;
    let (token, user_id) = if logged_in {
        let token = data
            .get("token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_TOKEN_MISSING"))?;
        let user_id = data
            .get("userid")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow::anyhow!("KUGOU_QR_USER_ID_MISSING"))?;
        (Some(token.to_owned()), Some(user_id.to_string()))
    } else {
        (None, None)
    };
    Ok(KugouQrPollResult {
        code: status,
        message: data
            .get("nickname")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        logged_in,
        scanned: Some(status == 2 || status == 4),
        expired: Some(status == 0),
        cookie: None,
        token,
        user_id,
    })
}

fn random_aes_key() -> String {
    let mut rng = rand::rng();
    (0..16)
        .map(|_| KUGOU_AES_KEY_CHARS[rng.random_range(0..KUGOU_AES_KEY_CHARS.len())] as char)
        .collect()
}

fn random_kugou_register_key() -> String {
    let mut rng = rand::rng();
    (0..6)
        .map(|_| {
            KUGOU_REGISTER_KEY_CHARS[rng.random_range(0..KUGOU_REGISTER_KEY_CHARS.len())] as char
        })
        .collect::<String>()
        .to_lowercase()
}

fn random_kugou_uuid() -> String {
    let mut rng = rand::rng();
    let mut part = || format!("{:04x}", rng.random_range(0..=u16::MAX));
    format!(
        "{}{}-{}-{}-{}-{}{}{}",
        part(),
        part(),
        part(),
        part(),
        part(),
        part(),
        part(),
        part()
    )
}

fn encrypt_token_params(token: &str, random_key: &str) -> anyhow::Result<String> {
    let plaintext = serde_json::json!({ "token": token }).to_string();
    let key_hash = md5_hex(random_key.as_bytes());
    let aes_key = key_hash.as_bytes();
    let iv = &key_hash[16..].as_bytes();
    let mut output = vec![0u8; plaintext.len() + 16];
    let encrypted = Aes256CbcEnc::new_from_slices(aes_key, iv)
        .map_err(|err| anyhow::anyhow!("KUGOU_TOKEN_AES_INIT_FAILED: {err}"))?
        .encrypt_padded_b2b_mut::<Pkcs7>(plaintext.as_bytes(), &mut output)
        .map_err(|err| anyhow::anyhow!("KUGOU_TOKEN_AES_FAILED: {err}"))?;
    Ok(crate::utils::cryptors::to_hex_lower(encrypted))
}

fn encrypt_kugou_rsa(plaintext: &str) -> anyhow::Result<String> {
    let modulus = BigUint::parse_bytes(KUGOU_RSA_MODULUS_HEX.as_bytes(), 16)
        .ok_or_else(|| anyhow::anyhow!("invalid Kugou RSA modulus"))?;
    let public_key = RsaPublicKey::new(modulus, BigUint::from(0x10001u32))
        .map_err(|err| anyhow::anyhow!("invalid Kugou RSA public key: {err}"))?;
    let key_size = public_key.size();
    let bytes = plaintext.as_bytes();
    if bytes.len() > key_size {
        anyhow::bail!("Kugou RSA plaintext is longer than the key block");
    }

    // Matches the bundled JS RSA helper: plaintext starts at byte zero and
    // the rest of the raw RSA block is zero-filled.
    let mut padded = vec![0u8; key_size];
    padded[..bytes.len()].copy_from_slice(bytes);
    let message = BigUint::from_bytes_be(&padded);
    if message >= *public_key.n() {
        anyhow::bail!("Kugou RSA plaintext block is not smaller than the modulus");
    }

    let encrypted = message.modpow(public_key.e(), public_key.n());
    let encrypted_bytes = encrypted.to_bytes_be();
    if encrypted_bytes.len() > key_size {
        anyhow::bail!("Kugou RSA encrypted block is longer than the key size");
    }

    let mut output = vec![0u8; key_size];
    output[key_size - encrypted_bytes.len()..].copy_from_slice(&encrypted_bytes);
    Ok(crate::utils::cryptors::to_hex_upper(&output))
}

fn response_cookie_header(response: &reqwest::Response) -> Option<String> {
    cookie_header_from_headers(response.headers())
}

fn cookie_header_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookies = headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .filter_map(|header| header.split(';').next())
        .map(str::trim)
        .filter(|cookie| !cookie.is_empty() && cookie.contains('='))
        .collect::<Vec<_>>();
    (!cookies.is_empty()).then(|| cookies.join("; "))
}

fn cookie_from_response_body(body: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(body).ok()?;
    ["cookie", "cookies"]
        .iter()
        .find_map(|key| value.get(key).and_then(cookie_value_from_json))
        .or_else(|| {
            value.get("data").and_then(|data| {
                ["cookie", "cookies"]
                    .iter()
                    .find_map(|key| data.get(key).and_then(cookie_value_from_json))
            })
        })
}

fn cookie_value_from_json(value: &Value) -> Option<String> {
    match value {
        Value::String(cookie) => cookie_header_from_text(cookie),
        Value::Array(values) => {
            let cookies = values
                .iter()
                .filter_map(Value::as_str)
                .filter_map(cookie_header_from_text)
                .collect::<Vec<_>>();
            (!cookies.is_empty()).then(|| cookies.join("; "))
        }
        _ => None,
    }
}

fn cookie_header_from_text(cookie: &str) -> Option<String> {
    let cookie = cookie.split(';').next()?.trim();
    (!cookie.is_empty() && cookie.contains('=')).then(|| cookie.to_owned())
}


fn current_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn current_time_seconds() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as u128)
        .unwrap_or_default()
}

fn md5_hex(value: &[u8]) -> String {
    format!("{:x}", Md5::digest(value))
}

fn md5_hex_upper(value: &[u8]) -> String {
    md5_hex(value).to_uppercase()
}

fn required_value(value: &str, error: &'static str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow::anyhow!(error));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MockKugouQrApi {
        poll: Mutex<Option<KugouQrPollResult>>,
    }

    #[async_trait]
    impl KugouQrLoginApi for MockKugouQrApi {
        async fn create_qr(&self) -> anyhow::Result<KugouQrCode> {
            Ok(KugouQrCode {
                key: "demo-key".to_owned(),
                image: "data:image/png;base64,demo".to_owned(),
                url: Some("https://www.kugou.com/".to_owned()),
            })
        }

        async fn check_qr(&self, _key: &str) -> anyhow::Result<KugouQrPollResult> {
            self.poll
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| anyhow::anyhow!("missing mock poll"))
        }
    }

    fn service(poll: KugouQrPollResult) -> KugouQrLoginService {
        create_kugou_qr_login_service(KugouQrLoginDeps {
            api: Box::new(MockKugouQrApi {
                poll: Mutex::new(Some(poll)),
            }),
        })
    }

    #[tokio::test]
    async fn creates_key_and_reads_cached_image() {
        let service = service(KugouQrPollResult::default());
        let key = service.create_key().await.unwrap();
        let image = service.create_image(&key.key).await.unwrap();

        assert_eq!(key.provider, ProviderId::Kugou);
        assert_eq!(image.img, "data:image/png;base64,demo");
        assert_eq!(image.url.as_deref(), Some("https://www.kugou.com/"));
    }

    #[tokio::test]
    async fn stores_cookie_after_successful_poll() {
        let service = service(KugouQrPollResult {
            code: 0,
            message: Some("ok".to_owned()),
            logged_in: true,
            scanned: Some(true),
            expired: Some(false),
            cookie: Some("KuGoo=userid%3D42%26token%3Ddemo".to_owned()),
            token: None,
            user_id: None,
        });
        let result = service.check("demo-key").await.unwrap();

        assert!(result.logged_in);
        assert_eq!(result.stored, Some(true));
        assert_eq!(result.provider, ProviderId::Kugou);
    }

    #[test]
    fn matches_the_web_qr_signature() {
        let params = BTreeMap::from([
            ("appid".to_owned(), "1014".to_owned()),
            ("clienttime".to_owned(), "1700000000000".to_owned()),
            ("clientver".to_owned(), "8131".to_owned()),
            ("dfid".to_owned(), "demo-dfid".to_owned()),
            ("mid".to_owned(), "demo-mid".to_owned()),
            ("plat".to_owned(), "4".to_owned()),
            ("qrcode_txt".to_owned(), KUGOU_QR_TEXT.to_owned()),
            ("srcappid".to_owned(), "2919".to_owned()),
            ("type".to_owned(), "1".to_owned()),
            ("uuid".to_owned(), "demo-mid".to_owned()),
        ]);

        assert_eq!(
            signature_web_qr(&params),
            "62A10DC33BC02CDF2B9400639B815F1D"
        );
    }

    #[test]
    fn matches_the_qr_poll_signature() {
        let device = KugouQrDevice {
            mid: "demo-mid".to_owned(),
            dfid: "demo-dfid".to_owned(),
            uuid: "demo-mid".to_owned(),
        };
        let params = build_check_params(1700000000123, &device, "demo-qrcode-1014");

        assert_eq!(
            params.get("signature").map(String::as_str),
            Some("04BCA01299A381DF4C1E06CBC2835FFE")
        );
    }

    #[test]
    fn matches_the_token_exchange_signature() {
        let params = BTreeMap::from([
            ("appid".to_owned(), "1014".to_owned()),
            ("clienttime".to_owned(), "1700000000".to_owned()),
            ("clienttime_ms".to_owned(), "1700000000123".to_owned()),
            ("clientver".to_owned(), "1000".to_owned()),
            ("dev".to_owned(), "web".to_owned()),
            ("dfid".to_owned(), "demo-dfid".to_owned()),
            ("mid".to_owned(), "demo-mid".to_owned()),
            ("params".to_owned(), "demo-params-payload".to_owned()),
            ("pk".to_owned(), "demo-pk-payload".to_owned()),
            ("plat".to_owned(), "4".to_owned()),
            ("srcappid".to_owned(), "2919".to_owned()),
            ("userid".to_owned(), "42".to_owned()),
            ("uuid".to_owned(), "demo-mid".to_owned()),
        ]);

        assert_eq!(
            signature_web_qr(&params),
            "F5C9941CC5B6B89EDE850ED3140DBBCB"
        );
    }

    #[test]
    fn token_exchange_crypto_fixture_is_valid() {
        let token = "kugou-test-token-placeholder";
        let aes_key = "A1B2C3D4E5F6G7H8";
        let clienttime_ms = 1785274163122u128;
        let params = encrypt_token_params(token, aes_key).unwrap();
        let pk_plaintext = serde_json::json!({
            "clienttime_ms": clienttime_ms,
            "key": aes_key,
        })
        .to_string();
        let pk = encrypt_kugou_rsa(&pk_plaintext).unwrap();

        assert!(!params.is_empty());
        assert_eq!(pk.len(), 256);
    }

    #[test]
    fn generates_web_shaped_uuid_before_deriving_mid() {
        let uuid = random_kugou_uuid();
        let parts = uuid.split('-').collect::<Vec<_>>();

        assert_eq!(
            parts.iter().map(|part| part.len()).collect::<Vec<_>>(),
            vec![8, 4, 4, 4, 12]
        );
        assert!(
            uuid.chars()
                .all(|character| character == '-' || character.is_ascii_hexdigit())
        );
        assert_eq!(md5_hex(uuid.as_bytes()).len(), 32);
    }

    #[test]
    fn keeps_only_the_first_cookie_segment_from_each_set_cookie_header() {
        let mut headers = HeaderMap::new();
        for value in [
            "KuGoo=KugooID=42&KugooPwd=demo-password&t=demo-token; expires=tomorrow; path=/;domain=.kugou.com",
            "KugooID=42; expires=tomorrow; path=/;domain=.kugou.com;HttpOnly",
            "t=demo-token; expires=tomorrow; path=/;domain=.kugou.com;HttpOnly",
            "a_id=1014; expires=Wed, 29-Jul-26 20:18:24 GMT; path=/;domain=.kugou.com;HttpOnly",
            "UserName=demo-user; expires=tomorrow; path=/;domain=.kugou.com;HttpOnly",
            "mid=demo-mid; expires=tomorrow; path=/;domain=.kugou.com;HttpOnly",
            "dfid=demo-dfid; expires=tomorrow; path=/;domain=.kugou.com;HttpOnly",
        ] {
            headers.append(SET_COOKIE, value.parse().unwrap());
        }

        assert_eq!(
            cookie_header_from_headers(&headers).as_deref(),
            Some(
                "KuGoo=KugooID=42&KugooPwd=demo-password&t=demo-token; KugooID=42; t=demo-token; a_id=1014; UserName=demo-user; mid=demo-mid; dfid=demo-dfid"
            )
        );
    }

    #[test]
    fn reads_cookie_array_from_token_response_body() {
        let cookie = cookie_from_response_body(
            r#"{"data":{"cookies":["KuGoo=KugooID=42; expires=tomorrow", "mid=demo-mid; path=/"]}}"#,
        );

        assert_eq!(cookie.as_deref(), Some("KuGoo=KugooID=42; mid=demo-mid"));
    }

    #[test]
    fn parses_qr_key_and_image_from_response() {
        let qr = parse_qr_response(serde_json::json!({
            "data": {
                "qrcode": "demo-qrcode-1014",
                "qrcode_img": "data:image/png;base64,demo"
            },
            "status": 1,
            "error_code": 0
        }))
        .unwrap();

        assert_eq!(qr.key, "demo-qrcode-1014");
        assert_eq!(qr.image, "data:image/png;base64,demo");
    }

    #[test]
    fn maps_qr_poll_states_and_login_payload() {
        let waiting = parse_qr_poll_response(serde_json::json!({
            "data": { "status": 1 },
            "status": 1,
            "error_code": 0
        }))
        .unwrap();
        assert_eq!(waiting.code, 1);
        assert_eq!(waiting.scanned, Some(false));
        assert_eq!(waiting.expired, Some(false));
        assert!(!waiting.logged_in);

        let logged_in = parse_qr_poll_response(serde_json::json!({
            "data": {
                "status": 4,
                "token": "demo-token",
                "userid": 42u64,
                "nickname": "demo"
            },
            "status": 1,
            "error_code": 0
        }))
        .unwrap();
        assert!(logged_in.logged_in);
        assert_eq!(logged_in.scanned, Some(true));
        assert_eq!(logged_in.cookie, None);
        assert_eq!(logged_in.token.as_deref(), Some("demo-token"));
        assert_eq!(logged_in.user_id.as_deref(), Some("42"));
    }
}
