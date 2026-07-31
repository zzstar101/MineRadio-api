use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::{Result, anyhow, bail};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    services::{
        auth_session::set_runtime_provider_cookie,
        qq_mqtt_login::{MqttLoginEvent, MqttLoginSession},
        qr_login::{QrLogin, QrSession, QrSessionStore},
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
    mqtt: MqttLoginSession,
    finished: bool,
}

#[derive(Default)]
pub struct QqMusicQrLoginService {
    deps: QqMusicQrLoginDeps,
    sessions: QrSessionStore<QqQrLoginSession>,
}

#[async_trait::async_trait]
impl QrLogin for QqMusicQrLoginService {
    async fn create_key(&self) -> Result<ProviderLoginQrKey> {
        let payload = self.music_api(create_qr_request()).await?;
        let data = payload
            .get("result")
            .and_then(|value| value.get("data"))
            .ok_or_else(|| anyhow!("QQ_MQTT_QR_RESPONSE_MISSING_DATA"))?;
        let key = required_string(data, "qrcodeID", "QQ_MQTT_QR_RESPONSE_MISSING_KEY")?;
        let image = required_string(data, "qrcode", "QQ_MQTT_QR_RESPONSE_MISSING_IMAGE")?;
        self.sessions
            .insert(
                key.clone(),
                image,
                QqQrLoginSession {
                    mqtt: MqttLoginSession::new(&key),
                    finished: false,
                },
            )
            .await;
        Ok(ProviderLoginQrKey {
            provider: ProviderId::Qq,
            key,
        })
    }

    async fn create_image(&self, key: &str) -> Result<ProviderLoginQrImage> {
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

    async fn check(&self, key: &str) -> Result<ProviderLoginQrCheck> {
        let key = required_key(key)?;
        let session = self.session(&key).await?;
        let mut session = session.lock().await;
        if session.state.finished {
            bail!("QQ_MQTT_QR_SESSION_FINISHED");
        }

        let event = session.state.mqtt.poll_event().await?;
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
            session.state.finished = true;
            drop(session);
            self.sessions.remove(&key).await;
        }
        response
    }
}

impl QqMusicQrLoginService {
    async fn session(&self, key: &str) -> Result<Arc<Mutex<QrSession<QqQrLoginSession>>>> {
        self.sessions
            .get(key)
            .await
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

        let data_map = flatten_data_to_map(data);

        let login_type = data_map
            .get("loginType")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("QQ_MQTT_LOGIN_RESPONSE_MISSING_LOGIN_TYPE"))?;

        match login_type {
            2 => {
                let cookie = cookie_from_data_map(&data_map)?;
                set_runtime_provider_cookie(ProviderId::Qq, cookie)
                    .await
                    .map_err(|error| anyhow!(error))?;
            }
            1 => {
                let wechat_body = build_wechat_exchange_request(&data_map);
                let wechat_payload = self.music_api(wechat_body).await?;

                let wechat_data = wechat_payload
                    .get("result")
                    .and_then(|v| v.get("data"))
                    .ok_or_else(|| anyhow!("QQ_MQTT_WECHAT_LOGIN_RESPONSE_MISSING_DATA"))?;
                let mut wechat_data_map = flatten_data_to_map(wechat_data);
                // 微信第二次兑换后特调：musicid -> wxuin + uin，musickey -> qm_keyst + qqmusic_key
                remap_wechat_key(&mut wechat_data_map, "musicid", "wxuin");
                remap_wechat_key(&mut wechat_data_map, "musicid", "uin");
                remap_wechat_key(&mut wechat_data_map, "musickey", "qm_keyst");
                remap_wechat_key(&mut wechat_data_map, "musickey", "qqmusic_key");
                let cookie = cookie_from_data_map(&wechat_data_map)?;
                set_runtime_provider_cookie(ProviderId::Qq, cookie)
                    .await
                    .map_err(|error| anyhow!(error))?;
            }
            _ => {
                bail!("QQ_MQTT_LOGIN_UNKNOWN_LOGIN_TYPE: {}", login_type);
            }
        }

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
        sessions: QrSessionStore::default(),
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

fn build_wechat_exchange_request(data_map: &HashMap<String, Value>) -> Value {
    let mut param = serde_json::Map::new();
    for (key, value) in data_map {
        param.insert(key.clone(), value.clone());
    }
    param.insert("strAppid".to_string(), json!("wx48db31d50e334801"));
    param.insert(
        "deviceName".to_string(),
        json!(format!("Mineradio{}", &get_guid()[..1])),
    );
    param.insert("deviceType".to_string(), json!("Widnows"));
    json!({
        "result": {
            "module": "music.login.LoginServer",
            "method": "Login",
            "param": param
        },
        "comm": {
            "ct": 19,
            "cv": 2201,
            "chid": "0",
            "guid": get_guid()
        }
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

fn flatten_data_to_map(data: &Value) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    if let Value::Object(obj) = data {
        for (key, value) in obj {
            match value {
                Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null => {
                    map.insert(key.clone(), value.clone());
                }
                _ => {} // skip nested objects and arrays
            }
        }
    }
    map
}

/// 微信第二次兑换后特调：将 source_key 的值复制到 target_key，若 source_key 存在。
fn remap_wechat_key(data_map: &mut HashMap<String, Value>, source_key: &str, target_key: &str) {
    if let Some(value) = data_map.get(source_key).cloned() {
        data_map.insert(target_key.to_string(), value);
    }
}

/// Build cookie string from a flattened data map.
/// Each non-empty base-type value becomes a `key=value` pair.
fn cookie_from_data_map(data_map: &HashMap<String, Value>) -> Result<String> {
    let mut parts: Vec<String> = Vec::new();
    for (key, value) in data_map {
        let s = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Null => continue,
            _ => continue,
        };
        if s.is_empty() {
            continue;
        }
        parts.push(format!("{key}={s}"));
    }
    if parts.is_empty() {
        bail!("QQ_MQTT_LOGIN_COOKIE_EMPTY");
    }
    Ok(parts.join("; "))
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
        let data = json!({
            "musicid": 10001,
            "musickey": "login-key",
            "loginType": 6
        });
        let data_map = flatten_data_to_map(&data);
        let cookie = cookie_from_data_map(&data_map).unwrap();
        // HashMap iteration order is non-deterministic — sort for stable assertion
        let mut parts: Vec<&str> = cookie.split("; ").collect();
        parts.sort();
        assert_eq!(
            parts.join("; "),
            "loginType=6; musicid=10001; musickey=login-key"
        );
    }
}
