use std::collections::HashMap;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Client, header::HeaderMap};

use crate::{
    providers::qq::sign::{gtk_from_pskey, hash33},
    services::auth_session::set_runtime_provider_cookie,
    types::{ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
};

type CookieMap = HashMap<String, String>;

#[derive(Clone, Debug)]
struct QqPtuiResult {
    code: i64,
    redirect_url: Option<String>,
    message: Option<String>,
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
    image_cache: tokio::sync::Mutex<HashMap<String, String>>,
}

const QQ_QR_SHOW_URL: &str = "https://ssl.ptlogin2.qq.com/ptqrshow?appid=716027609&e=2&l=M&s=3&d=72&v=4&t=0.9698127522807933&daid=383&pt_3rd_aid=100497308&u1=https%3A%2F%2Fgraph.qq.com%2Foauth2.0%2Flogin_jump";
const QQ_AUTHORIZE_URL: &str = "https://graph.qq.com/oauth2.0/authorize";
const QQ_MUSICU_URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const QQ_REDIRECT_URI: &str =
    "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/";

impl QqQrLoginService {
    pub async fn create_key(&self) -> anyhow::Result<ProviderLoginQrKey> {
        let resp = self
            .deps
            .client
            .get(QQ_QR_SHOW_URL)
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .send()
            .await?;
        let qrsig = read_set_cookie(resp.headers())
            .and_then(|header| {
                regex::Regex::new(r"qrsig=([^;]+)").ok().and_then(|re| {
                    re.captures(&header)
                        .and_then(|cap| cap.get(1).map(|m| m.as_str().to_owned()))
                })
            })
            .ok_or_else(|| anyhow::anyhow!("QQ_QR_SIG_MISSING"))?;
        let bytes = resp.bytes().await?;
        let img = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
        let key = encode_key(&qrsig, hash33(&qrsig));
        self.image_cache.lock().await.insert(key.clone(), img);
        Ok(ProviderLoginQrKey {
            provider: "qq".to_owned(),
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
            provider: "qq".to_owned(),
            key: normalized_key.to_owned(),
            img,
            url: None,
        })
    }

    pub async fn check(&self, key: &str) -> anyhow::Result<ProviderLoginQrCheck> {
        let normalized_key = key.trim();
        let decoded =
            decode_key(normalized_key).ok_or_else(|| anyhow::anyhow!("QQ_QR_KEY_REQUIRED"))?;
        let mut cookies = CookieMap::new();
        let check_resp = self
            .deps
            .client
            .get(check_url(now_millis(), &decoded.ptqrtoken))
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .header("cookie", format!("qrsig={}", decoded.qrsig))
            .send()
            .await?;
        merge_cookies(
            &mut cookies,
            read_set_cookie(check_resp.headers()).as_deref(),
        );
        let text = check_resp.text().await?;
        let ptui = parse_ptui_callback(&text);
        let message = normalize_poll_message(&ptui, &text);
        if ptui.code != 0 && !text.contains("登录成功") {
            let expired = ptui.code == 65 || message == "二维码已过期";
            if expired {
                self.image_cache.lock().await.remove(normalized_key);
            }
            return Ok(ProviderLoginQrCheck {
                provider: "qq".to_owned(),
                key: normalized_key.to_owned(),
                code: ptui.code,
                message: Some(message.clone()),
                logged_in: false,
                scanned: Some(ptui.code == 67 || message.starts_with("已扫码")),
                expired: Some(expired),
                stored: Some(false),
            });
        }

        let redirect_url = ptui
            .redirect_url
            .filter(|url| !url.is_empty())
            .ok_or_else(|| anyhow::anyhow!("QQ_QR_REDIRECT_MISSING"))?;
        let check_sig_resp = self
            .deps
            .client
            .get(redirect_url)
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .header("cookie", cookie_header(&cookies))
            .send()
            .await?;
        merge_cookies(
            &mut cookies,
            read_set_cookie(check_sig_resp.headers()).as_deref(),
        );

        let p_skey = cookie_value(&cookies, "p_skey");
        if p_skey.is_empty() {
            anyhow::bail!("QQ_QR_PSKEY_MISSING");
        }
        let gtk = gtk_from_pskey(&p_skey);
        let authorize_resp = self
            .deps
            .client
            .post(QQ_AUTHORIZE_URL)
            .timeout(Duration::from_millis(self.deps.timeout_ms))
            .header("cookie", cookie_header(&cookies))
            .form(&build_authorize_form(gtk))
            .send()
            .await?;
        merge_cookies(
            &mut cookies,
            read_set_cookie(authorize_resp.headers()).as_deref(),
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
            .header("cookie", cookie_header(&cookies))
            .body(build_musicu_body(gtk, &code))
            .send()
            .await?;
        merge_cookies(
            &mut cookies,
            read_set_cookie(musicu_resp.headers()).as_deref(),
        );
        let cookie = cookie_header(&cookies);
        if cookie.is_empty() {
            anyhow::bail!("QQ_QR_COOKIE_MISSING");
        }
        set_runtime_provider_cookie("qq".to_owned(), cookie)
            .await
            .map_err(|err| anyhow::anyhow!(err))?;
        self.image_cache.lock().await.remove(normalized_key);
        Ok(ProviderLoginQrCheck {
            provider: "qq".to_owned(),
            key: normalized_key.to_owned(),
            code: 0,
            message: Some("登录成功".to_owned()),
            logged_in: true,
            scanned: Some(true),
            expired: Some(false),
            stored: Some(true),
        })
    }
}

pub fn create_qq_qr_login_service(deps: QqQrLoginDeps) -> QqQrLoginService {
    QqQrLoginService {
        deps,
        image_cache: tokio::sync::Mutex::new(HashMap::new()),
    }
}

#[derive(Clone, Debug)]
struct DecodedKey {
    qrsig: String,
    ptqrtoken: String,
}

fn read_set_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn parse_set_cookie(header: Option<&str>) -> Vec<String> {
    header
        .unwrap_or_default()
        .split(',')
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

fn merge_cookies(cookies: &mut CookieMap, header: Option<&str>) {
    for cookie in parse_set_cookie(header) {
        if let Some((name, _)) = cookie.split_once('=') {
            cookies.insert(name.to_owned(), cookie);
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

fn encode_key(qrsig: &str, ptqrtoken: u32) -> String {
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
        message: values.get(4).cloned(),
    }
}

fn normalize_poll_message(result: &QqPtuiResult, text: &str) -> String {
    if result.code == 0 || text.contains("登录成功") {
        "登录成功".to_owned()
    } else if result.code == 65 || text.contains("已失效") {
        "二维码已过期".to_owned()
    } else if result.code == 67 || text.contains("认证中") || text.contains("已扫描") {
        "已扫码，请在手机上确认登录".to_owned()
    } else {
        "未扫描二维码".to_owned()
    }
}

fn build_authorize_form(gtk: u32) -> Vec<(&'static str, String)> {
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
        ("auth_time", format!("{:?}", std::time::SystemTime::now())),
        ("ui", default_guid()),
    ]
}

fn build_musicu_body(gtk: u32, code: &str) -> String {
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

fn check_url(now: i64, ptqrtoken: &str) -> String {
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
        .append_pair("js_ver", "23111510")
        .append_pair("js_type", "1")
        .append_pair(
            "login_sig",
            "du-YS1h8*0GqVqcrru0pXkpwVg2DYw-DtbFulJ62IgPf6vfiJe*4ONVrYc5hMUNE",
        )
        .append_pair("pt_uistyle", "40")
        .append_pair("aid", "716027609")
        .append_pair("daid", "383")
        .append_pair("pt_3rd_aid", "100497308")
        .append_pair("o1vId", "3674fc47871e9c407d8838690b355408")
        .append_pair("pt_js_version", "v1.48.1");
    format!("https://ssl.ptlogin2.qq.com/ptqrlogin?{}", params.finish())
}

fn extract_query_param(location: &str, name: &str) -> Option<String> {
    let url = url::Url::parse(location).ok()?;
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.to_string())
}

fn default_guid() -> String {
    use rand::Rng;

    let mut rng = rand::thread_rng();
    "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx"
        .chars()
        .map(|part| match part {
            'x' => format!("{:x}", rng.gen_range(0..16)),
            'y' => format!("{:x}", (rng.gen_range(0..16) & 3) | 8),
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
