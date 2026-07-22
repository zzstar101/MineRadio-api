use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Result, anyhow, bail};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    services::{
        auth_session::set_runtime_provider_cookie,
        qq_mqtt_login::{MqttLoginEvent, MqttLoginSession},
    },
    types::{ProviderId, ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
    utils::cryptors::qq::{get_guid, sign},
};

const QQ_MUSIC_API_URL: &str = "https://u.y.qq.com/cgi-bin/musics.fcg";
const QQ_MUSIC_REFERER: &str = "https://y.qq.com/";
const QQ_MUSIC_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; MSIE 9.0; Windows NT 6.1; WOW64; Trident/5.0)";

#[derive(Clone)]
pub struct QqMusicQrLoginDeps {
    pub client: Client,
    pub timeout_ms: u64,
}

impl Default for QqMusicQrLoginDeps {
    fn default() -> Self {
        Self {
            client: Client::new(),
            timeout_ms: 10_000,
        }
    }
}

struct QqQrLoginSession {
    image: String,
    mqtt: MqttLoginSession,
    finished: bool,
}

#[derive(Default)]
pub struct QqMusicQrLoginService {
    deps: QqMusicQrLoginDeps,
    sessions: Mutex<HashMap<String, Arc<Mutex<QqQrLoginSession>>>>,
}

impl QqMusicQrLoginService {
    pub async fn create_key(&self) -> Result<ProviderLoginQrKey> {
        let payload = self.music_api(create_qr_request()).await?;
        let data = payload
            .get("result")
            .and_then(|value| value.get("data"))
            .ok_or_else(|| anyhow!("QQ_MQTT_QR_RESPONSE_MISSING_DATA"))?;
        let key = required_string(data, "qrcodeID", "QQ_MQTT_QR_RESPONSE_MISSING_KEY")?;
        let image = required_string(data, "qrcode", "QQ_MQTT_QR_RESPONSE_MISSING_IMAGE")?;
        let session = Arc::new(Mutex::new(QqQrLoginSession {
            image,
            mqtt: MqttLoginSession::new(&key),
            finished: false,
        }));
        self.sessions.lock().await.insert(key.clone(), session);
        Ok(ProviderLoginQrKey {
            provider: ProviderId::Qq,
            key,
        })
    }

    pub async fn create_image(&self, key: &str) -> Result<ProviderLoginQrImage> {
        let key = required_key(key)?;
        let session = self.session(&key).await?;
        let image = session.lock().await.image.clone();
        Ok(ProviderLoginQrImage {
            provider: ProviderId::Qq,
            key,
            img: image,
            url: None,
        })
    }

    pub async fn check(&self, key: &str) -> Result<ProviderLoginQrCheck> {
        let key = required_key(key)?;
        let session = self.session(&key).await?;
        let mut session = session.lock().await;
        if session.finished {
            bail!("QQ_MQTT_QR_SESSION_FINISHED");
        }

        let event = session.mqtt.poll_event().await?;
        let terminal = event.is_terminal();
        let response = match event {
            MqttLoginEvent::WaitingScan => Ok(check_response(
                &key,
                66,
                "等待扫码",
                false,
                false,
                false,
                false,
            )),
            MqttLoginEvent::WaitingConfirm => Ok(check_response(
                &key,
                67,
                "已扫码，请在手机上确认登录",
                false,
                true,
                false,
                false,
            )),
            MqttLoginEvent::QrCodeExpired => Ok(check_response(
                &key,
                65,
                "二维码已过期",
                false,
                false,
                true,
                false,
            )),
            MqttLoginEvent::Canceled => Ok(check_response(
                &key,
                -1,
                "登录已取消",
                false,
                false,
                false,
                false,
            )),
            MqttLoginEvent::LoginFailed => Ok(check_response(
                &key,
                -1,
                "登录失败",
                false,
                false,
                false,
                false,
            )),
            MqttLoginEvent::Cookies {
                music_id,
                music_key,
            } => self.complete_mqtt_login(&key, &music_id, &music_key).await,
        };

        if terminal {
            session.finished = true;
            drop(session);
            self.sessions.lock().await.remove(&key);
        }
        response
    }

    async fn session(&self, key: &str) -> Result<Arc<Mutex<QqQrLoginSession>>> {
        self.sessions
            .lock()
            .await
            .get(key)
            .cloned()
            .ok_or_else(|| anyhow!("QQ_MQTT_QR_SESSION_MISSING"))
    }

    async fn complete_mqtt_login(
        &self,
        qrcode_id: &str,
        music_id: &str,
        music_key: &str,
    ) -> Result<ProviderLoginQrCheck> {
        let music_id = music_id
            .parse::<u64>()
            .map_err(|_| anyhow!("QQ_MQTT_LOGIN_INVALID_MUSIC_ID"))?;
        let payload = self
            .music_api(login_with_mqtt_ticket_request(
                qrcode_id, music_id, music_key,
            ))
            .await?;
        let data = payload
            .get("result")
            .and_then(|value| value.get("data"))
            .ok_or_else(|| anyhow!("QQ_MQTT_LOGIN_RESPONSE_MISSING_DATA"))?;
        let cookie = cookie_from_login_data(data)?;
        set_runtime_provider_cookie(ProviderId::Qq, cookie)
            .await
            .map_err(|error| anyhow!(error))?;
        Ok(check_response(
            qrcode_id,
            0,
            "登录成功",
            true,
            true,
            false,
            true,
        ))
    }

    async fn music_api(&self, body: Value) -> Result<Value> {
        let sign = sign(&serde_json::to_string(&body)?);
        self.deps
            .client
            .post(QQ_MUSIC_API_URL)
            .query(&[("sign", sign)])
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .header("referer", QQ_MUSIC_REFERER)
            .header("user-agent", QQ_MUSIC_USER_AGENT)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<Value>()
            .await
            .map_err(Into::into)
    }
}

pub fn create_qqmusic_qr_login_service(deps: QqMusicQrLoginDeps) -> QqMusicQrLoginService {
    QqMusicQrLoginService {
        deps,
        sessions: Mutex::new(HashMap::new()),
    }
}

fn create_qr_request() -> Value {
    json!({
        "result": {
            "module": "music.login.LoginServer",
            "method": "CreateQRCode",
            "param": { "tmeAppID": "qqmusic", "ct": 19, "cv": 2201 }
        },
        "comm": { "ct": 19, "cv": 2201, "chid": "0", "guid": get_guid() }
    })
}

fn login_with_mqtt_ticket_request(qrcode_id: &str, music_id: u64, music_key: &str) -> Value {
    json!({
        "result": {
            "module": "music.login.LoginServer",
            "method": "Login",
            "param": {
                "musicid": music_id,
                "qrCodeID": qrcode_id,
                "token": music_key
            }
        },
        "comm": { "ct": 19, "cv": 2201, "chid": "0", "guid": get_guid(), "tmeLoginType": 6 }
    })
}

fn required_key(key: &str) -> Result<String> {
    let key = key.trim();
    if key.is_empty() {
        bail!("QQ_QR_KEY_REQUIRED");
    }
    Ok(key.to_owned())
}

fn required_string(data: &Value, field: &str, error: &'static str) -> Result<String> {
    data.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!(error))
}

fn required_scalar_string(data: &Value, field: &str, error: &'static str) -> Result<String> {
    data.get(field)
        .and_then(|value| match value {
            Value::String(value) => Some(value.trim().to_owned()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!(error))
}

fn cookie_from_login_data(data: &Value) -> Result<String> {
    let mut parts: Vec<String> = Vec::new();

    // Required fields
    let music_id =
        required_scalar_string(data, "musicid", "QQ_MQTT_LOGIN_RESPONSE_MISSING_MUSIC_ID")?;
    let music_key =
        required_scalar_string(data, "musickey", "QQ_MQTT_LOGIN_RESPONSE_MISSING_MUSIC_KEY")?;
    let login_type = required_scalar_string(
        data,
        "loginType",
        "QQ_MQTT_LOGIN_RESPONSE_MISSING_LOGIN_TYPE",
    )?;

    // Core credentials
    parts.push(format!("uin={music_id}"));
    parts.push(format!("qqmusic_key={music_key}"));
    parts.push(format!("qm_keyst={music_key}"));
    parts.push(format!("tmeLoginType={login_type}"));
    parts.push(format!("login_type={login_type}"));

    // Token & refresh
    append_if_non_empty(data, &mut parts, "access_token", "psrf_qqaccess_token");
    append_if_non_empty(data, &mut parts, "refresh_token", "psrf_qqrefresh_token");
    append_if_non_empty(data, &mut parts, "refresh_key", "refresh_key");

    // Encrypted user id
    append_if_non_empty(data, &mut parts, "encryptUin", "euin");

    // TTL / expiry
    append_if_non_zero(data, &mut parts, "keyExpiresIn", "key_expires_in");
    append_if_non_zero(data, &mut parts, "expired_at", "expired_at");

    Ok(parts.join("; "))
}

fn append_if_non_empty(data: &Value, parts: &mut Vec<String>, json_key: &str, cookie_key: &str) {
    if let Some(value) = data.get(json_key).and_then(Value::as_str).map(str::trim) {
        if !value.is_empty() {
            parts.push(format!("{cookie_key}={value}"));
        }
    }
}

fn append_if_non_zero(data: &Value, parts: &mut Vec<String>, json_key: &str, cookie_key: &str) {
    if let Some(value) = data.get(json_key) {
        let s = match value {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.trim().to_owned(),
            _ => return,
        };
        if s != "0" && !s.is_empty() {
            parts.push(format!("{cookie_key}={s}"));
        }
    }
}

fn check_response(
    key: &str,
    code: i64,
    message: &str,
    logged_in: bool,
    scanned: bool,
    expired: bool,
    stored: bool,
) -> ProviderLoginQrCheck {
    ProviderLoginQrCheck {
        provider: ProviderId::Qq,
        key: key.to_owned(),
        code,
        message: Some(message.to_owned()),
        logged_in,
        scanned: Some(scanned),
        expired: Some(expired),
        stored: Some(stored),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_qr_request_has_required_protocol_fields() {
        let request = create_qr_request();
        assert_eq!(request["result"]["module"], "music.login.LoginServer");
        assert_eq!(request["result"]["method"], "CreateQRCode");
        assert_eq!(request["result"]["param"]["tmeAppID"], "qqmusic");
        assert!(
            request["comm"]["guid"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
    }

    #[test]
    fn maps_scanned_state_to_existing_api_shape() {
        let response = check_response(
            "key",
            67,
            "已扫码，请在手机上确认登录",
            false,
            true,
            false,
            false,
        );
        assert_eq!(response.code, 67);
        assert_eq!(response.scanned, Some(true));
        assert!(!response.logged_in);
    }

    #[test]
    fn login_ticket_exchange_matches_the_source_protocol() {
        let request = login_with_mqtt_ticket_request("qr-id", 10001, "event-key");
        assert_eq!(request["result"]["method"], "Login");
        assert_eq!(request["result"]["param"]["qrCodeID"], "qr-id");
        assert_eq!(request["result"]["param"]["token"], "event-key");
        assert_eq!(request["comm"]["tmeLoginType"], 6);
    }

    #[test]
    fn login_exchange_produces_the_complete_qq_cookie() {
        let cookie = cookie_from_login_data(&json!({
            "musicid": 10001,
            "musickey": "login-key",
            "loginType": 6
        }))
        .unwrap();
        assert_eq!(
            cookie,
            "uin=10001; qqmusic_key=login-key; qm_keyst=login-key; tmeLoginType=6; login_type=6"
        );
    }
}
