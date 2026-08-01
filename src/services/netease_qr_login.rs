use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use qrcode_generator::{QrCodeEcc, to_svg_to_string};
use reqwest::{
    Client,
    header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    services::auth_session::set_runtime_provider_cookie,
    services::qr_login::QrLogin,
    types::{ProviderId, ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
    utils::{encrypt_weapi, generate_weapi_secret_key},
};

const NETEASE_DOMAIN: &str = "https://music.163.com";
const NETEASE_QR_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0";

#[derive(Clone, Debug, Default)]
pub struct NeteaseApiResponse {
    pub body: Option<Value>,
    pub cookie: Option<Value>,
}

#[async_trait]
pub trait NeteaseApiCall: Send + Sync {
    async fn call(&self, query: Value) -> anyhow::Result<NeteaseApiResponse>;
}

pub struct NeteaseQrLoginService {
    deps: NeteaseQrLoginDeps,
}

pub struct NeteaseQrLoginDeps {
    pub qr_key: Box<dyn NeteaseApiCall>,
    pub qr_create: Box<dyn NeteaseApiCall>,
    pub qr_check: Box<dyn NeteaseApiCall>,
    pub now: Option<Box<dyn Fn() -> i64 + Send + Sync>>,
}

#[async_trait]
impl QrLogin for NeteaseQrLoginService {
    async fn create_key(&self) -> anyhow::Result<ProviderLoginQrKey> {
        let resp = self
            .deps
            .qr_key
            .call(serde_json::json!({ "timestamp": self.now() }))
            .await?;
        let key = read_string(response_body(&resp).get("unikey"))
            .ok_or_else(|| anyhow::anyhow!("NETEASE_QR_KEY_MISSING"))?;
        Ok(ProviderLoginQrKey {
            provider: ProviderId::Netease,
            key,
        })
    }

    async fn create_image(&self, key: &str) -> anyhow::Result<ProviderLoginQrImage> {
        let normalized_key = key.trim();
        if normalized_key.is_empty() {
            anyhow::bail!("NETEASE_QR_KEY_REQUIRED");
        }
        let resp = self
            .deps
            .qr_create
            .call(serde_json::json!({
                "key": normalized_key,
                "qrimg": true,
                "timestamp": self.now()
            }))
            .await?;
        let data = response_data(&resp);
        let img = read_string(data.get("qrimg"))
            .ok_or_else(|| anyhow::anyhow!("NETEASE_QR_IMAGE_MISSING"))?;
        Ok(ProviderLoginQrImage {
            provider: ProviderId::Netease,
            key: normalized_key.to_owned(),
            img,
            url: read_string(data.get("qrurl")),
        })
    }

    async fn check(&self, key: &str) -> anyhow::Result<ProviderLoginQrCheck> {
        let normalized_key = key.trim();
        if normalized_key.is_empty() {
            anyhow::bail!("NETEASE_QR_KEY_REQUIRED");
        }
        let mut resp = self
            .deps
            .qr_check
            .call(serde_json::json!({
                "key": normalized_key,
                "noCookie": true,
                "timestamp": self.now()
            }))
            .await?;
        let mut cookie = read_qr_cookie(&resp);
        let code = read_qr_code(&resp);
        if code == 803 && cookie.is_none() {
            resp = self
                .deps
                .qr_check
                .call(serde_json::json!({
                    "key": normalized_key,
                    "timestamp": self.now()
                }))
                .await?;
            cookie = read_qr_cookie(&resp);
        }

        let stored = code == 803 && cookie.is_some();
        if let Some(cookie) = cookie.filter(|_| stored) {
            set_runtime_provider_cookie(ProviderId::Netease, cookie)
                .await
                .map_err(|err| anyhow::anyhow!(err))?;
        }

        Ok(ProviderLoginQrCheck {
            provider: ProviderId::Netease,
            key: normalized_key.to_owned(),
            code,
            message: read_qr_message(&resp),
            logged_in: stored,
            scanned: Some(code == 802),
            expired: Some(code == 800),
            stored: Some(stored),
        })
    }
}

impl NeteaseQrLoginService {
    fn now(&self) -> i64 {
        self.deps.now.as_ref().map(|now| now()).unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis() as i64)
                .unwrap_or(0)
        })
    }
}

pub fn create_netease_qr_login_service(deps: NeteaseQrLoginDeps) -> NeteaseQrLoginService {
    NeteaseQrLoginService { deps }
}

pub fn create_netease_qr_login_service_with_client(client: Client) -> NeteaseQrLoginService {
    create_netease_qr_login_service(NeteaseQrLoginDeps {
        qr_key: Box::new(NeteaseQrKeyCall {
            client: client.clone(),
        }),
        qr_create: Box::new(NeteaseQrCreateCall {}),
        qr_check: Box::new(NeteaseQrCheckCall { client }),
        now: None,
    })
}

fn as_obj(value: Option<&Value>) -> Option<&serde_json::Map<String, Value>> {
    value.and_then(Value::as_object)
}

fn response_body(resp: &NeteaseApiResponse) -> serde_json::Map<String, Value> {
    as_obj(resp.body.as_ref()).cloned().unwrap_or_default()
}

fn response_data(resp: &NeteaseApiResponse) -> serde_json::Map<String, Value> {
    response_body(resp)
        .get("data")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn read_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn read_number(value: Option<&Value>) -> Option<i64> {
    value.and_then(Value::as_i64)
}

fn read_qr_cookie(resp: &NeteaseApiResponse) -> Option<String> {
    let body = response_body(resp);
    let data = response_data(resp);
    read_string(resp.cookie.as_ref())
        .or_else(|| read_string(body.get("cookie")))
        .or_else(|| read_string(data.get("cookie")))
        .or_else(|| read_string(data.get("cookies")))
}

fn read_qr_code(resp: &NeteaseApiResponse) -> i64 {
    let body = response_body(resp);
    let data = response_data(resp);
    read_number(body.get("code"))
        .or_else(|| read_number(data.get("code")))
        .unwrap_or(0)
}

fn read_qr_message(resp: &NeteaseApiResponse) -> Option<String> {
    let body = response_body(resp);
    let data = response_data(resp);
    read_string(body.get("message")).or_else(|| read_string(data.get("message")))
}

struct NeteaseQrKeyCall {
    client: Client,
}

#[async_trait]
impl NeteaseApiCall for NeteaseQrKeyCall {
    async fn call(&self, _query: Value) -> anyhow::Result<NeteaseApiResponse> {
        request_qr_response(
            &self.client,
            "/api/login/qrcode/unikey",
            serde_json::json!({ "type": 3 }),
        )
        .await
    }
}

struct NeteaseQrCreateCall {}

#[async_trait]
impl NeteaseApiCall for NeteaseQrCreateCall {
    async fn call(&self, query: Value) -> anyhow::Result<NeteaseApiResponse> {
        let key = query
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("NETEASE_QR_KEY_REQUIRED"))?;
        let url = format!("https://music.163.com/login?codekey={key}");
        let include_image = query.get("qrimg").and_then(Value::as_bool).unwrap_or(false);
        let image = if include_image {
            render_qr_data_uri(&url)?
        } else {
            String::new()
        };
        Ok(NeteaseApiResponse {
            body: Some(serde_json::json!({
                "code": 200,
                "data": {
                    "qrurl": url,
                    "qrimg": image,
                }
            })),
            cookie: None,
        })
    }
}

struct NeteaseQrCheckCall {
    client: Client,
}

#[async_trait]
impl NeteaseApiCall for NeteaseQrCheckCall {
    async fn call(&self, query: Value) -> anyhow::Result<NeteaseApiResponse> {
        let key = query
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("NETEASE_QR_KEY_REQUIRED"))?;
        request_qr_response(
            &self.client,
            "/api/login/qrcode/client/login",
            serde_json::json!({ "key": key, "type": 3 }),
        )
        .await
    }
}

async fn request_qr_response(
    client: &Client,
    uri: &str,
    payload: Value,
) -> anyhow::Result<NeteaseApiResponse> {
    let mut body = payload.as_object().cloned().unwrap_or_default();
    body.insert("csrf_token".to_owned(), Value::String(String::new()));
    let encrypted = encrypt_weapi(&Value::Object(body), Some(&generate_weapi_secret_key()))
        .map_err(|err| anyhow::anyhow!("NETEASE_QR_ENCRYPT_FAILED: {err}"))?;

    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(NETEASE_QR_USER_AGENT));
    headers.insert(REFERER, HeaderValue::from_static(NETEASE_DOMAIN));
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers.insert(COOKIE, HeaderValue::from_str(&qr_cookie_header())?);
    let response = client
        .post(format!(
            "{NETEASE_DOMAIN}/weapi/{}",
            uri.trim_start_matches("/api/")
        ))
        .headers(headers)
        .form(&[
            ("params", encrypted.params),
            ("encSecKey", encrypted.enc_sec_key),
        ])
        .send()
        .await?
        .error_for_status()?;
    let cookie = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    let body = response.json::<Value>().await?;
    Ok(NeteaseApiResponse {
        body: Some(body),
        cookie: (!cookie.is_empty()).then_some(Value::String(cookie)),
    })
}

fn qr_cookie_header() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let seed = format!("netease{timestamp:x}");
    format!(
        "__remember_me=true; _ntes_nuid={seed}; _ntes_nnid={seed},{timestamp}; WEVNSM=1.0.0; WNMCID={}.{}.01.0; appver=3.1.17.204416; channel=netease; os=pc; osver=Microsoft-Windows-10-Professional-build-19045-64bit",
        &seed[..6.min(seed.len())],
        timestamp
    )
}

fn render_qr_data_uri(url: &str) -> anyhow::Result<String> {
    let svg = to_svg_to_string(url, QrCodeEcc::Medium, 256, None::<&str>)
        .map_err(|err| anyhow::anyhow!("failed to render netease qr image: {err}"))?;
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        BASE64.encode(svg.as_bytes())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct MockCall {
        responses: Mutex<VecDeque<NeteaseApiResponse>>,
    }

    impl MockCall {
        fn new(responses: Vec<NeteaseApiResponse>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    #[async_trait]
    impl NeteaseApiCall for MockCall {
        async fn call(&self, _query: Value) -> anyhow::Result<NeteaseApiResponse> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("missing mock response"))
        }
    }

    fn service(
        qr_key: Box<dyn NeteaseApiCall>,
        qr_create: Box<dyn NeteaseApiCall>,
        qr_check: Box<dyn NeteaseApiCall>,
    ) -> NeteaseQrLoginService {
        create_netease_qr_login_service(NeteaseQrLoginDeps {
            qr_key,
            qr_create,
            qr_check,
            now: Some(Box::new(|| 123)),
        })
    }

    #[tokio::test]
    async fn create_image_returns_qr_payload() {
        let service = service(
            Box::new(MockCall::new(vec![])),
            Box::new(NeteaseQrCreateCall {}),
            Box::new(MockCall::new(vec![])),
        );

        let image = service.create_image("demo-key").await.unwrap();

        assert_eq!(image.provider.as_str(), "netease");
        assert_eq!(image.key, "demo-key");
        assert_eq!(
            image.url.as_deref(),
            Some("https://music.163.com/login?codekey=demo-key")
        );
        assert!(image.img.starts_with("data:image/svg+xml;base64,"));
    }

    #[tokio::test]
    async fn check_retries_and_marks_cookie_stored() {
        let service = service(
            Box::new(MockCall::new(vec![])),
            Box::new(MockCall::new(vec![])),
            Box::new(MockCall::new(vec![
                NeteaseApiResponse {
                    body: Some(serde_json::json!({
                        "code": 803,
                        "message": "ok"
                    })),
                    cookie: None,
                },
                NeteaseApiResponse {
                    body: Some(serde_json::json!({
                        "code": 803,
                        "cookie": "MUSIC_U=demo"
                    })),
                    cookie: Some(Value::String("MUSIC_U=demo".to_owned())),
                },
            ])),
        );

        let result = service.check("demo-key").await.unwrap();

        assert_eq!(result.provider.as_str(), "netease");
        assert_eq!(result.key, "demo-key");
        assert_eq!(result.code, 803);
        assert!(result.logged_in);
        assert_eq!(result.stored, Some(true));
    }
}
