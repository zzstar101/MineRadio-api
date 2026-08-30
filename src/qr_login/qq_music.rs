use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    auth_session::set_runtime_provider_cookie,
    providers::qq::transport::qq_post_model,
    qr_login::{
        QrLogin, QrSession, QrSessionStore,
        common::{
            check_qq_login_error, check_response, flatten_data_to_map, normalize_login_cookie,
            required_key,
        },
        mqtt::{MqttLoginEvent, MqttLoginSession},
    },
    types::{ProviderId, ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
    utils::{
        cookie::Cookie,
        cryptors::qq::{default_qq_cookie, x5},
    },
};

const MQTT_LOGIN_REQUEST_KEY: &str = "music.login.LoginServer.Login";

#[derive(Clone)]
pub struct QqMusicQrLoginDeps {
    pub client: Client,
}

impl Default for QqMusicQrLoginDeps {
    fn default() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

struct QqQrLoginSession {
    mqtt: MqttLoginSession,
    guid: String,
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
        let guid = x5();
        let cookie = default_qq_cookie(Some(&guid), None);
        let payload = self
            .music_api(create_qr_request(), cookie, "mqtt_create_qr")
            .await?;
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
                    guid,
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
        let key = required_key(key, "QQ_QR_KEY_REQUIRED")?;
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
        let key = required_key(key, "QQ_QR_KEY_REQUIRED")?;
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
            } => {
                let guid = session.state.guid.clone();
                self.complete_mqtt_login(&key, &music_id, &music_key, &guid)
                    .await
            }
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
        guid: &str,
    ) -> Result<ProviderLoginQrCheck> {
        let music_id = music_id
            .parse::<u64>()
            .map_err(|_| anyhow!("QQ_MQTT_LOGIN_INVALID_MUSIC_ID"))?;
        let initial = default_qq_cookie(Some(guid), None);
        let mut request_cookie = initial.clone();
        request_cookie.insert("tmeLoginType", "6");
        let payload = self
            .music_api(
                login_with_mqtt_ticket_request(qrcode_id, music_id, music_key),
                request_cookie,
                "mqtt_login",
            )
            .await?;

        check_qq_login_error(&payload)?;
        let data = payload
            .get(MQTT_LOGIN_REQUEST_KEY)
            .and_then(|value| value.get("data"))
            .ok_or_else(|| anyhow!("QQ_MQTT_LOGIN_RESPONSE_MISSING_DATA"))?;
        let data_map = flatten_data_to_map(data);
        let login_type = data_map
            .get("loginType")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("QQ_MQTT_LOGIN_RESPONSE_MISSING_LOGIN_TYPE"))?;

        let initial: String = initial.into();
        match login_type {
            2 => {
                let cookie =
                    normalize_login_cookie(data, &initial, false, "QQ_MQTT_LOGIN_COOKIE_EMPTY")?;
                set_runtime_provider_cookie(ProviderId::Qq, cookie)
                    .await
                    .map_err(|error| anyhow!(error))?;
            }
            1 => {
                let cookie =
                    normalize_login_cookie(data, &initial, true, "QQ_MQTT_LOGIN_COOKIE_EMPTY")?;
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

    async fn music_api(&self, body: Value, cookie: Cookie, action: &'static str) -> Result<Value> {
        qq_post_model(self.deps.client.clone(), body, None, cookie, action, true)
            .await
            .map_err(|err| anyhow!("{err}"))
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
        }
    })
}

fn login_with_mqtt_ticket_request(qrcode_id: &str, music_id: u64, music_key: &str) -> Value {
    json!({
        "music.login.LoginServer.Login": {
            "module": "music.login.LoginServer",
            "method": "Login",
            "param": {
                "musicid": music_id,
                "qrCodeID": qrcode_id,
                "token": music_key
            }
        }
    })
}

fn required_string(data: &Value, field: &str, error: &'static str) -> Result<String> {
    data.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!(error))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::qr_login::common::{
        cookie_from_data_map, cookie_with_qqmusic_guid, remap_qq_login_data_map,
    };

    use super::*;

    #[test]
    fn create_qr_request_has_required_protocol_fields() {
        let request = create_qr_request();
        assert_eq!(request["result"]["module"], "music.login.LoginServer");
        assert_eq!(request["result"]["method"], "CreateQRCode");
        assert_eq!(request["result"]["param"]["tmeAppID"], "qqmusic");
        assert!(
            request.get("comm").is_none(),
            "comm 应由 transport 自动构建"
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
        assert_eq!(request[MQTT_LOGIN_REQUEST_KEY]["method"], "Login");
        assert_eq!(request[MQTT_LOGIN_REQUEST_KEY]["param"]["musicid"], 10001);
        assert_eq!(
            request[MQTT_LOGIN_REQUEST_KEY]["param"]["qrCodeID"],
            "qr-id"
        );
        assert_eq!(
            request[MQTT_LOGIN_REQUEST_KEY]["param"]["token"],
            "event-key"
        );
        assert!(
            request.get("comm").is_none(),
            "comm 应由 transport 自动构建"
        );
    }

    #[test]
    fn login_exchange_produces_the_complete_qq_cookie() {
        let data = json!({
            "musicid": 10001,
            "musickey": "login-key",
            "loginType": 6
        });
        let data_map = flatten_data_to_map(&data);
        let cookie = cookie_from_data_map(&data_map, "QQ_MQTT_LOGIN_COOKIE_EMPTY").unwrap();
        // HashMap iteration order is non-deterministic — sort for stable assertion
        let mut parts: Vec<&str> = cookie.split("; ").collect();
        parts.sort();
        assert_eq!(
            parts.join("; "),
            "loginType=6; musicid=10001; musickey=login-key"
        );
    }

    #[test]
    fn login_cookie_persists_the_session_guid() {
        // 契约: 调用方传入整段初始 cookie 字符串, 函数按原样追加到登录 cookie
        assert_eq!(
            cookie_with_qqmusic_guid("uin=10001".to_owned(), "qqmusic_guid=session-guid"),
            "uin=10001; qqmusic_guid=session-guid"
        );
    }

    #[test]
    fn normalize_login_cookie_uses_the_shared_response_formatter() {
        let cookie = normalize_login_cookie(
            &json!({ "musicid": 10001, "musickey": "login-key" }),
            "qqmusic_guid=session-guid",
            true,
            "QQ_LOGIN_COOKIE_EMPTY",
        )
        .unwrap();
        let mut parts: Vec<_> = cookie.split("; ").collect();
        parts.sort();
        assert_eq!(
            parts.join("; "),
            "musicid=10001; musickey=login-key; qm_keyst=login-key; \
             qqmusic_guid=session-guid; qqmusic_key=login-key; qqmusic_uin=10001; \
             tmeLoginType=1; uin=10001; wxuin=10001"
        );
    }

    #[test]
    fn mqtt_wechat_first_exchange_preserves_the_continuation_fields() {
        let cookie = normalize_login_cookie(
            &json!({
                "loginType": 1,
                "musicid": 1152921505274451474u64,
                "musickey": "music-key",
                "refresh_key": "refresh-key"
            }),
            "session-guid",
            true,
            "QQ_MQTT_LOGIN_COOKIE_EMPTY",
        )
        .unwrap();
        let map = cookie
            .split("; ")
            .filter_map(|entry| entry.split_once('='))
            .collect::<HashMap<_, _>>();

        assert_eq!(map["tmeLoginType"], "1");
        assert_eq!(map["musicid"], "1152921505274451474");
        assert_eq!(map["qm_keyst"], "music-key");
        assert_eq!(map["refresh_key"], "refresh-key");
    }

    #[test]
    fn login_exchange_remaps_all_cookie_keys_for_each_platform() {
        let mut data = flatten_data_to_map(&json!({
            "access_token": "access",
            "openid": "openid",
            "unionid": "unionid",
            "refresh_token": "refresh",
            "expired_at": 123,
            "musickey": "music-key",
            "encryptUin": "encrypted",
            "musicid": 10001
        }));

        remap_qq_login_data_map(&mut data, false);

        assert_eq!(data["psrf_qqaccess_token"], json!("access"));
        assert_eq!(data["psrf_qqopenid"], json!("openid"));
        assert_eq!(data["psrf_qqunionid"], json!("unionid"));
        assert_eq!(data["psrf_qqrefresh_token"], json!("refresh"));
        assert_eq!(data["psrf_access_token_expiresAt"], json!(123));
        assert_eq!(data["qm_keyst"], json!("music-key"));
        assert_eq!(data["euin"], json!("encrypted"));
        assert_eq!(data["uin"], json!(10001));
        assert!(!data.contains_key("wxuin"));

        remap_qq_login_data_map(&mut data, true);
        assert_eq!(data["wxuin"], json!(10001));
    }

    #[test]
    fn rejects_qq_login_device_limit_error_tip() {
        let payload = json!({
            "result": {
                "data": {
                    "errTip": "超出登录设备数量限制"
                }
            }
        });

        let error = check_qq_login_error(&payload).unwrap_err().to_string();
        assert!(error.contains("QQ_LOGIN_DEVICE_LIMIT"));
    }
}
