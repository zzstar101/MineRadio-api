use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Client,
    header::{HeaderMap, SET_COOKIE},
};

use crate::{
    services::auth_session::set_runtime_provider_cookie,
    types::{ProviderId, ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
    utils::cryptors::qq::{get_guid, gtk_from_pskey},
};

type CookieMap = HashMap<String, String>;

#[derive(Clone, Debug)]
struct QqPtuiResult {
    code: i64,
    redirect_url: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum QqQrPollState {
    Waiting,
    Authenticating,
    Success,
    Expired,
    Unknown,
}

#[derive(Clone)]
pub struct QqQrLoginDeps {
    pub client: Client,
    pub timeout_ms: u64,
}

impl Default for QqQrLoginDeps {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_else(|_| Client::new()),
            timeout_ms: 10_000,
        }
    }
}

#[derive(Default)]
pub struct QqQrLoginService {
    deps: QqQrLoginDeps,
    image_cache: Arc<tokio::sync::Mutex<HashMap<String, String>>>,
    sessions: Arc<tokio::sync::Mutex<HashMap<String, QqQrLoginSession>>>,
    cleanup_running: Arc<tokio::sync::Mutex<bool>>,
}

#[derive(Clone)]
struct QqQrLoginSession {
    cookies: CookieMap,
    login_sig: String,
    device_id: String,
    created_at: Instant,
}

const QQ_XLOGIN_URL: &str = "https://xui.ptlogin2.qq.com/cgi-bin/xlogin";
const QQ_QR_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/53.0.47.134 Safari/537.36 QBCore/3.53.47.400 QQBrowser/9.0.2524.400 pcqqmusic/22.30.3563.0626 SkinId/10001|00cc65|0|1|||1fd4af";
const QQ_QR_SHOW_URL: &str = "https://xui.ptlogin2.qq.com/ssl/ptqrshow";
const QQ_AUTHORIZE_URL: &str = "https://graph.qq.com/oauth2.0/authorize";
const QQ_MUSICU_URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const QQ_REDIRECT_URI: &str =
    "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/";
const QQ_QR_SESSION_TTL: Duration = Duration::from_secs(30);

impl QqQrLoginService {
    pub async fn create_key(&self) -> anyhow::Result<ProviderLoginQrKey> {
        let xlogin_resp = self
            .deps
            .client
            .get(xlogin_url())
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .send()
            .await?
            .error_for_status()?;
        let mut cookies = CookieMap::new();
        merge_cookies(&mut cookies, read_set_cookies(xlogin_resp.headers()));
        let login_sig = cookie_value(&cookies, "pt_login_sig");
        if login_sig.is_empty() {
            anyhow::bail!("QQ_QR_LOGIN_SIG_MISSING");
        }

        let qr_resp = self
            .deps
            .client
            .get(qr_show_url())
            .header("cookie", cookie_header(&cookies))
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .send()
            .await?
            .error_for_status()?;
        merge_cookies(&mut cookies, read_set_cookies(qr_resp.headers()));
        let qrsig = read_set_cookies(qr_resp.headers())
            .find_map(|header| {
                regex::Regex::new(r"qrsig=([^;]+)").ok().and_then(|re| {
                    re.captures(&header)
                        .and_then(|cap| cap.get(1).map(|m| m.as_str().to_owned()))
                })
            })
            .ok_or_else(|| anyhow::anyhow!("QQ_QR_SIG_MISSING"))?;
        let bytes = qr_resp.bytes().await?;
        let img = format!("data:image/png;base64,{}", STANDARD.encode(bytes));

        // 注意：这里必须和网页版当前 hash33 算法保持一致；check 的 ptqrtoken 依赖它。
        let hash = |t: &str| -> i32 {
            let mut e = 0i32;

            for c in t.chars() {
                e = e.wrapping_add((e << 5).wrapping_add(c as i32));
            }

            e & 0x7fffffff
        };
        let key = encode_key(&qrsig, hash(&qrsig) as u64);
        self.image_cache.lock().await.insert(key.clone(), img);
        self.sessions.lock().await.insert(
            key.clone(),
            QqQrLoginSession {
                cookies,
                login_sig,
                device_id: get_guid(),
                created_at: Instant::now(),
            },
        );
        self.ensure_cleanup_task().await;
        Ok(ProviderLoginQrKey {
            provider: ProviderId::Qq,
            key,
        })
    }

    pub async fn create_image(&self, key: &str) -> anyhow::Result<ProviderLoginQrImage> {
        let normalized_key = key.trim();
        if decode_key(normalized_key).is_none() {
            anyhow::bail!("QQ_QR_KEY_REQUIRED");
        }
        let img = self
            .image_cache
            .lock()
            .await
            .get(normalized_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("QQ_QR_IMAGE_MISSING"))?;
        Ok(ProviderLoginQrImage {
            provider: ProviderId::Qq,
            key: normalized_key.to_owned(),
            img,
            url: None,
        })
    }

    pub async fn check(&self, key: &str) -> anyhow::Result<ProviderLoginQrCheck> {
        let normalized_key = key.trim();
        let decoded =
            decode_key(normalized_key).ok_or_else(|| anyhow::anyhow!("QQ_QR_KEY_REQUIRED"))?;
        let mut session = self
            .sessions
            .lock()
            .await
            .get(normalized_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("QQ_QR_SESSION_MISSING"))?;
        if cookie_value(&session.cookies, "qrsig") != decoded.qrsig {
            anyhow::bail!("QQ_QR_SESSION_SIG_MISMATCH");
        }
        let url = check_url(
            now_millis(),
            &decoded.ptqrtoken,
            &session.login_sig,
            &session.device_id,
        );
        let check_resp = self
            .deps
            .client
            .get(&url)
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .header("cookie", cookie_header(&session.cookies))
            .header("referer", xlogin_url())
            .header("user-agent", QQ_QR_USER_AGENT)
            .send()
            .await?;
        merge_cookies(&mut session.cookies, read_set_cookies(check_resp.headers()));
        let text = check_resp.text().await?;
        let ptui = parse_ptui_callback(&text);
        let state = classify_poll_state(&ptui, &text);
        let message = normalize_poll_message(&ptui, &text);
        match state {
            QqQrPollState::Success => {}
            QqQrPollState::Expired => {
                self.image_cache.lock().await.remove(normalized_key);
                self.sessions.lock().await.remove(normalized_key);
                return Ok(ProviderLoginQrCheck {
                    provider: ProviderId::Qq,
                    key: normalized_key.to_owned(),
                    code: ptui.code,
                    message: Some(message),
                    logged_in: false,
                    scanned: Some(false),
                    expired: Some(true),
                    stored: Some(false),
                });
            }
            QqQrPollState::Waiting | QqQrPollState::Authenticating => {
                self.sessions
                    .lock()
                    .await
                    .insert(normalized_key.to_owned(), session);
                self.ensure_cleanup_task().await;
                return Ok(ProviderLoginQrCheck {
                    provider: ProviderId::Qq,
                    key: normalized_key.to_owned(),
                    code: ptui.code,
                    message: Some(message),
                    logged_in: false,
                    scanned: Some(state == QqQrPollState::Authenticating),
                    expired: Some(false),
                    stored: Some(false),
                });
            }
            QqQrPollState::Unknown => {
                self.sessions
                    .lock()
                    .await
                    .insert(normalized_key.to_owned(), session);
                self.ensure_cleanup_task().await;
                anyhow::bail!("QQ_QR_UNKNOWN_POLL_CODE_{}", ptui.code);
            }
        }

        let redirect_url = ptui
            .redirect_url
            .filter(|url| !url.is_empty())
            .ok_or_else(|| anyhow::anyhow!("QQ_QR_REDIRECT_MISSING"))?;
        let check_sig_resp = self
            .deps
            .client
            .get(&redirect_url)
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .header("cookie", cookie_header(&session.cookies))
            .send()
            .await?;
        merge_cookies(
            &mut session.cookies,
            read_set_cookies(check_sig_resp.headers()),
        );
        let p_skey = cookie_value(&session.cookies, "p_skey");
        if p_skey.is_empty() {
            anyhow::bail!("QQ_QR_PSKEY_MISSING");
        }
        let gtk = gtk_from_pskey(&p_skey);
        let authorize_resp = self
            .deps
            .client
            .post(QQ_AUTHORIZE_URL)
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .header("cookie", cookie_header(&session.cookies))
            .form(&build_authorize_form(gtk))
            .send()
            .await?;
        merge_cookies(
            &mut session.cookies,
            read_set_cookies(authorize_resp.headers()),
        );
        let status = authorize_resp.status();
        let location = authorize_resp
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let code = extract_query_param(&location, "code").unwrap_or_default();
        if !status.is_redirection() || code.is_empty() {
            anyhow::bail!("QQ_QR_AUTHORIZE_FAILED");
        }

        let musicu_resp = self
            .deps
            .client
            .post(QQ_MUSICU_URL)
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .header("content-type", "application/x-www-form-urlencoded")
            .header("cookie", cookie_header(&session.cookies))
            .body(build_musicu_body(gtk, &code))
            .send()
            .await?;
        merge_cookies(
            &mut session.cookies,
            read_set_cookies(musicu_resp.headers()),
        );
        let cookie = cookie_header(&session.cookies);
        if cookie.is_empty() {
            anyhow::bail!("QQ_QR_COOKIE_MISSING");
        }
        set_runtime_provider_cookie(ProviderId::Qq, cookie)
            .await
            .map_err(|err| anyhow::anyhow!(err))?;
        self.image_cache.lock().await.remove(normalized_key);
        self.sessions.lock().await.remove(normalized_key);
        Ok(ProviderLoginQrCheck {
            provider: ProviderId::Qq,
            key: normalized_key.to_owned(),
            code: 0,
            message: Some("登录成功".to_owned()),
            logged_in: true,
            scanned: Some(true),
            expired: Some(false),
            stored: Some(true),
        })
    }

    async fn ensure_cleanup_task(&self) {
        let mut running = self.cleanup_running.lock().await;
        if *running {
            return;
        }
        *running = true;

        let sessions = Arc::clone(&self.sessions);
        let image_cache = Arc::clone(&self.image_cache);
        let cleanup_running = Arc::clone(&self.cleanup_running);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(QQ_QR_SESSION_TTL).await;

                let (expired_keys, empty) = {
                    let mut sessions = sessions.lock().await;
                    let now = Instant::now();
                    let expired_keys = sessions
                        .iter()
                        .filter(|(_, session)| {
                            now.duration_since(session.created_at) >= QQ_QR_SESSION_TTL
                        })
                        .map(|(key, _)| key.clone())
                        .collect::<Vec<_>>();
                    for key in &expired_keys {
                        sessions.remove(key);
                    }
                    (expired_keys, sessions.is_empty())
                };

                if !expired_keys.is_empty() {
                    let mut image_cache = image_cache.lock().await;
                    for key in expired_keys {
                        image_cache.remove(&key);
                    }
                }
                if empty {
                    let mut running = cleanup_running.lock().await;
                    if sessions.lock().await.is_empty() {
                        *running = false;
                        break;
                    }
                }
            }
        });
    }
}

pub fn create_qq_qr_login_service(deps: QqQrLoginDeps) -> QqQrLoginService {
    QqQrLoginService {
        deps,
        image_cache: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        sessions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        cleanup_running: Arc::new(tokio::sync::Mutex::new(false)),
    }
}

#[derive(Clone, Debug)]
struct DecodedKey {
    qrsig: String,
    ptqrtoken: String,
}

fn read_set_cookies(headers: &HeaderMap) -> impl Iterator<Item = &str> {
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
}

fn parse_set_cookie(header: &str) -> Vec<String> {
    split_set_cookie_header(header)
        .into_iter()
        .map(|part| part.split(';').next().unwrap_or_default().trim().to_owned())
        .filter(|part| {
            part.contains('=')
                && part
                    .split('=')
                    .nth(1)
                    .is_some_and(|value| !value.is_empty())
        })
        .collect()
}

fn split_set_cookie_header(header: &str) -> Vec<&str> {
    let mut cookies = Vec::new();
    let mut start = 0;

    for (index, character) in header.char_indices() {
        if character != ',' {
            continue;
        }
        let next = header[index + character.len_utf8()..].trim_start();
        let candidate = next.split([';', ',']).next().unwrap_or_default().trim();
        if !candidate.is_empty() && candidate.contains('=') {
            cookies.push(header[start..index].trim());
            start = index + character.len_utf8();
        }
    }

    cookies.push(header[start..].trim());
    cookies
}

fn merge_cookies<'a>(cookies: &mut CookieMap, headers: impl IntoIterator<Item = &'a str>) {
    for header in headers {
        for cookie in parse_set_cookie(header) {
            if let Some((name, _)) = cookie.split_once('=') {
                cookies.insert(name.to_owned(), cookie);
            }
        }
    }
}

fn cookie_value(cookies: &CookieMap, name: &str) -> String {
    cookies
        .get(name)
        .and_then(|pair| {
            pair.strip_prefix(&format!("{name}="))
                .map(ToOwned::to_owned)
        })
        .unwrap_or_default()
}

fn cookie_header(cookies: &CookieMap) -> String {
    cookies.values().cloned().collect::<Vec<_>>().join("; ")
}

fn encode_key(qrsig: &str, ptqrtoken: u64) -> String {
    format!("{}|{}", urlencoding::encode(qrsig), ptqrtoken)
}

fn decode_key(key: &str) -> Option<DecodedKey> {
    let (encoded_qrsig, ptqrtoken) = key.split_once('|')?;
    if encoded_qrsig.is_empty() || ptqrtoken.is_empty() {
        return None;
    }
    let qrsig = urlencoding::decode(encoded_qrsig).ok()?.to_string();
    Some(DecodedKey {
        qrsig,
        ptqrtoken: ptqrtoken.to_owned(),
    })
}

fn parse_ptui_callback(text: &str) -> QqPtuiResult {
    let values = regex::Regex::new(r"'([^']*)'")
        .ok()
        .map(|re| {
            re.captures_iter(text)
                .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_owned()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    QqPtuiResult {
        code: values
            .first()
            .and_then(|value| value.parse().ok())
            .unwrap_or(-1),
        redirect_url: values.get(2).cloned(),
    }
}

fn normalize_poll_message(result: &QqPtuiResult, text: &str) -> String {
    match classify_poll_state(result, text) {
        QqQrPollState::Waiting => "二维码未失效".to_owned(),
        QqQrPollState::Authenticating => "二维码认证中".to_owned(),
        QqQrPollState::Success => "登录成功！".to_owned(),
        QqQrPollState::Expired => "二维码已失效".to_owned(),
        QqQrPollState::Unknown => format!("二维码状态未知(code={})", result.code),
    }
}

fn classify_poll_state(result: &QqPtuiResult, text: &str) -> QqQrPollState {
    match result.code {
        66 => QqQrPollState::Waiting,
        67 => QqQrPollState::Authenticating,
        0 => QqQrPollState::Success,
        65 => QqQrPollState::Expired,
        _ if text.contains("二维码未失效") => QqQrPollState::Waiting,
        _ if text.contains("二维码认证中") => QqQrPollState::Authenticating,
        _ if text.contains("登录成功") => QqQrPollState::Success,
        _ if text.contains("二维码已失效") => QqQrPollState::Expired,
        _ => QqQrPollState::Unknown,
    }
}

fn build_authorize_form(gtk: u64) -> Vec<(&'static str, String)> {
    vec![
        ("response_type", "code".to_owned()),
        ("client_id", "100497308".to_owned()),
        ("redirect_uri", QQ_REDIRECT_URI.to_owned()),
        ("scope", "get_user_info,get_app_friends".to_owned()),
        ("state", "state".to_owned()),
        ("switch", String::new()),
        ("from_ptlogin", "1".to_owned()),
        ("src", "1".to_owned()),
        ("update_auth", "1".to_owned()),
        ("openapi", "1010_1030".to_owned()),
        ("g_tk", gtk.to_string()),
        ("auth_time", now_millis().to_string()),
        ("ui", default_guid()),
    ]
}

fn build_musicu_body(gtk: u64, code: &str) -> String {
    serde_json::json!({
        "comm": { "g_tk": gtk, "platform": "yqq", "ct": 24, "cv": 0 },
        "req": {
            "module": "QQConnectLogin.LoginServer",
            "method": "QQLogin",
            "param": { "code": code }
        }
    })
    .to_string()
}

fn xlogin_url() -> String {
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params
        .append_pair("appid", "716027609")
        .append_pair("daid", "383")
        .append_pair("style", "33")
        .append_pair("login_text", "登录")
        .append_pair("hide_title_bar", "1")
        .append_pair("hide_border", "1")
        .append_pair("target", "self")
        .append_pair("s_url", "https://graph.qq.com/oauth2.0/login_jump")
        .append_pair("pt_3rd_aid", "100497308")
        .append_pair(
            "pt_feedback_link",
            "https://support.qq.com/products/77942?customInfo=.appid100497308",
        )
        .append_pair("theme", "2")
        .append_pair("verify_theme", "");
    format!("{QQ_XLOGIN_URL}?{}", params.finish())
}

fn qr_show_url() -> String {
    use rand::RngExt;

    let mut rng = rand::rng();
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params
        .append_pair("appid", "716027609")
        .append_pair("e", "2")
        .append_pair("l", "M")
        .append_pair("s", "3")
        .append_pair("d", "72")
        .append_pair("v", "4")
        .append_pair("t", &rng.random::<f64>().to_string())
        .append_pair("daid", "383")
        .append_pair("pt_3rd_aid", "100497308")
        .append_pair("u1", "https://graph.qq.com/oauth2.0/login_jump");
    format!("{QQ_QR_SHOW_URL}?{}", params.finish())
}

fn check_url(now: i64, ptqrtoken: &str, login_sig: &str, device_id: &str) -> String {
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params
        .append_pair("u1", "https://graph.qq.com/oauth2.0/login_jump")
        .append_pair("ptqrtoken", ptqrtoken)
        .append_pair("ptredirect", "0")
        .append_pair("h", "1")
        .append_pair("t", "1")
        .append_pair("g", "1")
        .append_pair("from_ui", "1")
        .append_pair("ptlang", "2052")
        .append_pair("action", &format!("0-0-{now}"))
        .append_pair("js_ver", "26071711")
        .append_pair("js_type", "1")
        .append_pair("login_sig", login_sig)
        .append_pair("pt_uistyle", "40")
        .append_pair("aid", "716027609")
        .append_pair("daid", "383")
        .append_pair("pt_3rd_aid", "100497308")
        .append_pair("o1vId", device_id)
        .append_pair("pt_js_version", "c1987b96");
    format!(
        "https://xui.ptlogin2.qq.com/ssl/ptqrlogin?{}",
        params.finish()
    )
}

fn extract_query_param(location: &str, name: &str) -> Option<String> {
    let url = url::Url::parse(location).ok()?;
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.to_string())
}

fn default_guid() -> String {
    use rand::RngExt;

    let mut rng = rand::rng();
    "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx"
        .chars()
        .map(|part| match part {
            'x' => format!("{:x}", rng.random_range(0..16)),
            'y' => format!("{:x}", (rng.random_range(0..16) & 3) | 8),
            other => other.to_string(),
        })
        .collect::<String>()
        .to_uppercase()
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;

    #[test]
    fn merge_cookies_reads_every_set_cookie_header() {
        let mut headers = HeaderMap::new();
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static("ptcz=first; Expires=Wed, 21 Oct 2015 07:28:00 GMT; Path=/"),
        );
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static("p_skey=needed; Path=/"),
        );
        headers.append(SET_COOKIE, HeaderValue::from_static("u_key=final; Path=/"));

        let mut cookies = CookieMap::new();
        merge_cookies(&mut cookies, read_set_cookies(&headers));

        assert_eq!(cookie_value(&cookies, "ptcz"), "first");
        assert_eq!(cookie_value(&cookies, "p_skey"), "needed");
        assert_eq!(cookie_value(&cookies, "u_key"), "final");
    }

    #[test]
    fn check_url_uses_the_session_login_sig() {
        let url = check_url(1, "qr-token", "session-login-sig", "device-id");
        assert!(url.contains("ptqrtoken=qr-token"));
        assert!(url.contains("login_sig=session-login-sig"));
        assert!(url.contains("o1vId=device-id"));
    }

    #[test]
    fn xlogin_url_contains_the_qr_login_parameters() {
        let url = xlogin_url();
        assert!(url.contains("appid=716027609"));
        assert!(url.contains("daid=383"));
        assert!(url.contains("pt_3rd_aid=100497308"));
    }

    #[test]
    fn authorize_form_uses_numeric_auth_time() {
        let auth_time = build_authorize_form(1)
            .into_iter()
            .find(|(name, _)| *name == "auth_time")
            .map(|(_, value)| value)
            .expect("auth_time should be present");
        assert!(auth_time.parse::<i64>().is_ok());
    }

    #[test]
    fn poll_codes_map_to_explicit_states_and_messages() {
        let cases = [
            (66, QqQrPollState::Waiting, "二维码未失效"),
            (67, QqQrPollState::Authenticating, "二维码认证中"),
            (0, QqQrPollState::Success, "登录成功！"),
            (65, QqQrPollState::Expired, "二维码已失效"),
        ];

        for (code, state, message) in cases {
            let result = QqPtuiResult {
                code,
                redirect_url: None,
            };
            assert_eq!(classify_poll_state(&result, ""), state);
            assert_eq!(normalize_poll_message(&result, ""), message);
        }
    }

    #[test]
    fn poll_text_is_only_a_fallback_for_unknown_codes() {
        let result = QqPtuiResult {
            code: -1,
            redirect_url: None,
        };
        assert_eq!(
            classify_poll_state(&result, "二维码认证中。"),
            QqQrPollState::Authenticating
        );
        assert_eq!(
            classify_poll_state(&result, "unexpected response"),
            QqQrPollState::Unknown
        );
    }
}
