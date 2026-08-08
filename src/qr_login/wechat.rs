use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use regex::Regex;
use reqwest::{Client, header::LOCATION};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::{
    auth_session::set_runtime_provider_cookie,
    qr_login::common::{
        check_qq_login_error, check_response as check, normalize_login_cookie,
        qq_music_device_name, required_key,
    },
    qr_login::{QrLogin, QrSession, QrSessionStore},
    types::{ProviderId, ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
    utils::cryptors::qq::{x5, x9},
};

const WECHAT_QR_CONNECT_URL: &str = "\
https://open.weixin.qq.com/connect/qrconnect\
?appid=wx48db31d50e334801\
&redirect_uri=https%3A%2F%2Fy.qq.com%2Fwk_v17%2Fcommon_login.html%3Ftype%3DWX%26%26redirect%3D\
&response_type=code\
&scope=snsapi_login,snsapi_runtime_pcsdk\
&state=STATE\
&href=https%3A%2F%2Fy.qq.com%2Fmediastyle%2Fmusic_v17%2Fsrc%2Fcss%2Fpopup_wechat.css%23wechat_redirect\
&self_redirect=true\
&fast_login=0\
&clear=1";

const WECHAT_QRCODE_BASE: &str = "https://open.weixin.qq.com/connect/qrcode/";
const WECHAT_POLL_BASE: &str = "https://long.open.weixin.qq.com/connect/l/qrconnect?uuid=";
const WECHAT_QQ_REDIRECT_BASE: &str =
    "https://y.qq.com/portal/wx_redirect.html?login_type=2&surl=https://y.qq.com/&code=";
const WECHAT_POLL_TIMEOUT: Duration = Duration::from_secs(16);
const QQ_MUSIC_API_URL: &str = "https://u.y.qq.com/cgi-bin/musics.fcg";
const QQ_MUSIC_REFERER: &str = "https://y.qq.com/";
const QQ_MUSIC_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; MSIE 9.0; Windows NT 6.1; WOW64; Trident/5.0)";

static WECHAT_UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"/connect/qrcode/([a-zA-Z0-9]{16,64})"#).expect("compile wechat uuid regex")
});

static WECHAT_POLL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"window\.(\w+)\s*=\s*(-?\w+|"[^"]*"?|'[^']*'?)"#)
        .expect("compile wechat poll regex")
});

#[derive(Clone)]
pub struct WechatQrLoginDeps {
    pub client: Client,
    pub timeout_ms: u64,
}

impl Default for WechatQrLoginDeps {
    fn default() -> Self {
        Self {
            client: Client::new(),
            timeout_ms: 10_000,
        }
    }
}

struct WechatQrSession {
    uuid: String,
    guid: String,
    finished: bool,
    last_status_code: Option<i64>,
}

#[derive(Default)]
pub struct WechatQrLoginService {
    deps: WechatQrLoginDeps,
    sessions: QrSessionStore<WechatQrSession>,
}

#[async_trait::async_trait]
impl QrLogin for WechatQrLoginService {
    async fn create_key(&self) -> Result<ProviderLoginQrKey> {
        let resp = self
            .get_following_redirects(
                WECHAT_QR_CONNECT_URL.to_owned(),
                Duration::from_millis(self.deps.timeout_ms),
                Some("Mozilla/5.0 (compatible; MSIE 9.0; Windows NT 6.1; WOW64; Trident/5.0)"),
            )
            .await?;
        let html = resp.text().await?;

        let request_uuid = WECHAT_UUID_RE
            .captures(&html)
            .and_then(|cap| cap.get(1))
            .map(|m| m.as_str().to_owned())
            .ok_or_else(|| anyhow!("WECHAT_QR_UUID_NOT_FOUND"))?;

        let qrcode_url = format!("{}{}", WECHAT_QRCODE_BASE, request_uuid);
        let img_bytes = self
            .deps
            .client
            .get(&qrcode_url)
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let img = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(img_bytes)
        );

        let session = WechatQrSession {
            uuid: request_uuid.clone(),
            guid: x5(),
            finished: false,
            last_status_code: None,
        };
        self.sessions
            .insert(request_uuid.clone(), img, session)
            .await;

        Ok(ProviderLoginQrKey {
            provider: ProviderId::Qq,
            key: request_uuid,
        })
    }

    async fn create_image(&self, key: &str) -> Result<ProviderLoginQrImage> {
        let key = required_key(key, "WECHAT_QR_KEY_REQUIRED")?;
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
        let key = required_key(key, "WECHAT_QR_KEY_REQUIRED")?;
        let session = self.session(&key).await?;
        let mut session = session.lock().await;
        if session.state.finished {
            bail!("WECHAT_QR_SESSION_FINISHED");
        }

        let poll_url = match session.state.last_status_code {
            Some(404) => format!("{}{}&last=404", WECHAT_POLL_BASE, session.state.uuid),
            _ => format!("{}{}", WECHAT_POLL_BASE, session.state.uuid),
        };

        let text = self
            .deps
            .client
            .get(&poll_url)
            .timeout(WECHAT_POLL_TIMEOUT)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        let kv = parse_wechat_poll_response(&text);

        let status_code: i64 = kv
            .get("wx_errcode")
            .and_then(|v| v.parse().ok())
            .unwrap_or(-1);
        session.state.last_status_code = Some(status_code);

        let terminal = matches!(status_code, 402 | 405);
        let response = match status_code {
            // 等待扫描
            408 => check(&key, 66, "等待扫码", false, false, false, false),

            // 已扫描
            404 => check(
                &key,
                67,
                "已扫码，请在手机上确认登录",
                false,
                true,
                false,
                false,
            ),

            // 已过期
            402 => check(&key, 65, "二维码已过期", false, false, true, false),

            // 扫码成功（登录成功）
            405 => {
                let token = kv
                    .get("wx_code")
                    .cloned()
                    .ok_or_else(|| anyhow!("WECHAT_QR_TOKEN_MISSING"))?;
                let uuid = session.state.uuid.clone();
                let guid = session.state.guid.clone();
                self.confirm_wechat_login(&uuid).await?;
                self.complete_wechat_login(&key, &token, &guid).await?
            }

            _ => check(&key, -1, "未知状态", false, false, false, false),
        };

        if terminal {
            session.state.finished = true;
            drop(session);
            self.sessions.remove(&key).await;
        }

        Ok(response)
    }
}

impl WechatQrLoginService {
    async fn session(&self, key: &str) -> Result<Arc<Mutex<QrSession<WechatQrSession>>>> {
        self.sessions
            .get(key)
            .await
            .ok_or_else(|| anyhow!("WECHAT_QR_SESSION_MISSING"))
    }

    async fn complete_wechat_login(
        &self,
        key: &str,
        code: &str,
        guid: &str,
    ) -> Result<ProviderLoginQrCheck> {
        self.open_wechat_redirect(code).await?;
        let payload = self.music_api(wechat_login_request(code, guid)).await?;
        check_qq_login_error(&payload)?;
        let data = payload
            .get("WXLogin")
            .and_then(|value| value.get("data"))
            .ok_or_else(|| anyhow!("WECHAT_QR_LOGIN_RESPONSE_MISSING_DATA"))?;
        let cookie = normalize_login_cookie(data, guid, true, "WECHAT_QR_LOGIN_COOKIE_EMPTY")?;
        set_runtime_provider_cookie(ProviderId::Qq, cookie)
            .await
            .map_err(|error| anyhow!(error))?;

        Ok(check(key, 0, "登录成功", true, true, false, true))
    }

    async fn confirm_wechat_login(&self, uuid: &str) -> Result<()> {
        self.deps
            .client
            .get(format!("{}{}&last=404", WECHAT_POLL_BASE, uuid))
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    async fn open_wechat_redirect(&self, code: &str) -> Result<()> {
        self.get_following_redirects(
            wechat_redirect_url(code),
            Duration::from_millis(self.deps.timeout_ms),
            None,
        )
        .await?;
        Ok(())
    }

    async fn get_following_redirects(
        &self,
        mut url: String,
        timeout: Duration,
        user_agent: Option<&str>,
    ) -> Result<reqwest::Response> {
        for _ in 0..5 {
            let mut request = self.deps.client.get(&url).timeout(timeout);
            if let Some(user_agent) = user_agent {
                request = request.header("user-agent", user_agent);
            }
            let response = request.send().await?;
            if !response.status().is_redirection() {
                return Ok(response.error_for_status()?);
            }
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| anyhow!("WECHAT_QR_REDIRECT_LOCATION_MISSING"))?;
            url = url::Url::parse(&url)?.join(location)?.to_string();
        }
        bail!("WECHAT_QR_TOO_MANY_REDIRECTS")
    }

    async fn music_api(&self, body: Value) -> Result<Value> {
        let sign = x9(&serde_json::to_string(&body)?);
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

pub fn create_wechat_qr_login_service(deps: WechatQrLoginDeps) -> WechatQrLoginService {
    WechatQrLoginService {
        deps,
        sessions: QrSessionStore::default(),
    }
}

fn wechat_login_request(code: &str, guid: &str) -> Value {
    let device_name = qq_music_device_name();
    json!({
        "WXLogin": {
            "module": "music.login.LoginServer",
            "method": "Login",
            "param": {
                "code": code,
                "deviceName": device_name,
                "deviceType": "Widnows",
                "forceRefreshToken": 0,
                "onlyNeedAccessToken": 0,
                "strAppid": "wx48db31d50e334801"
            }
        },
        "comm": {
            "ct": 19,
            "cv": 2201,
            "chid": "0",
            "guid": guid,
            "tmeAppID": "qqmusic",
            "tmeLoginType": 1,
            "uin": "0",
            "wid": "4810302018970526720"
        }
    })
}

fn wechat_redirect_url(code: &str) -> String {
    format!("{WECHAT_QQ_REDIRECT_BASE}{code}&state=STATE")
}

fn parse_wechat_poll_response(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for cap in WECHAT_POLL_RE.captures_iter(text) {
        let key = cap.get(1).map(|m| m.as_str().to_owned());
        let val = cap.get(2).map(|m| {
            let raw = m.as_str();
            raw.trim_matches(&['"', '\''] as &[_]).to_owned()
        });
        if let (Some(k), Some(v)) = (key, val) {
            map.insert(k, v);
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qr_login::common::{
        cookie_from_data_map, flatten_data_to_map, remap_qq_login_data_map,
    };

    #[test]
    fn poll_parser_extracts_waiting_state() {
        let text = r#"window.wx_errcode=408;window.wx_code='';"#;
        let kv = parse_wechat_poll_response(text);
        assert_eq!(kv.get("wx_errcode").map(|v| v.as_str()), Some("408"));
        assert_eq!(kv.get("wx_code").map(|v| v.as_str()), Some(""));
    }

    #[test]
    fn poll_parser_extracts_scanned_state() {
        let text = r#"window.wx_errcode=404;window.wx_code='';"#;
        let kv = parse_wechat_poll_response(text);
        assert_eq!(kv.get("wx_errcode").map(|v| v.as_str()), Some("404"));
    }

    #[test]
    fn poll_parser_extracts_expired_state() {
        let text = r#"window.wx_errcode=402;window.wx_code='';"#;
        let kv = parse_wechat_poll_response(text);
        assert_eq!(kv.get("wx_errcode").map(|v| v.as_str()), Some("402"));
    }

    #[test]
    fn poll_parser_extracts_success_with_token() {
        let text = r#"window.wx_errcode=405;window.wx_code='071abc123def456';"#;
        let kv = parse_wechat_poll_response(text);
        assert_eq!(kv.get("wx_errcode").map(|v| v.as_str()), Some("405"));
        assert_eq!(
            kv.get("wx_code").map(|v| v.as_str()),
            Some("071abc123def456")
        );
    }

    #[test]
    fn poll_parser_handles_double_quotes_too() {
        let text = r#"window.wx_errcode=408;window.wx_errmsg="";"#;
        let kv = parse_wechat_poll_response(text);
        assert_eq!(kv.get("wx_errcode").map(|v| v.as_str()), Some("408"));
        assert_eq!(kv.get("wx_errmsg").map(|v| v.as_str()), Some(""));
    }

    #[test]
    fn wechat_login_request_uses_the_confirmed_exchange_shape() {
        let request = wechat_login_request("wx-code", "session-guid");
        assert_eq!(request["WXLogin"]["module"], "music.login.LoginServer");
        assert_eq!(request["WXLogin"]["method"], "Login");
        assert_eq!(request["WXLogin"]["param"]["code"], "wx-code");
        assert_eq!(request["comm"]["guid"], "session-guid");
        assert_eq!(
            request["WXLogin"]["param"]["deviceName"],
            qq_music_device_name()
        );
        assert_eq!(
            request["WXLogin"]["param"]["strAppid"],
            "wx48db31d50e334801"
        );
    }

    #[test]
    fn wechat_redirect_uses_the_login_code() {
        assert_eq!(
            wechat_redirect_url("wx-code"),
            "https://y.qq.com/portal/wx_redirect.html?login_type=2&surl=https://y.qq.com/&code=wx-code&state=STATE"
        );
    }

    #[test]
    fn exchange_data_remaps_wechat_cookie_keys() {
        let mut data = flatten_data_to_map(&json!({
            "musicid": 10001,
            "musickey": "login-key"
        }));
        remap_qq_login_data_map(&mut data, true);
        let cookie = cookie_from_data_map(&data, "WECHAT_QR_LOGIN_COOKIE_EMPTY").unwrap();
        let mut parts: Vec<_> = cookie.split("; ").collect();
        parts.sort();
        assert_eq!(
            parts.join("; "),
            "musicid=10001; musickey=login-key; qm_keyst=login-key; tmeLoginType=1; uin=10001; wxuin=10001"
        );
    }
}
