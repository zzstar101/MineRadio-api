use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use qrcode_generator::{QrCodeEcc, to_svg_to_string};
use reqwest::{
    Client,
    header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde_json::Value;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    auth_session::set_runtime_provider_cookie,
    qr_login::QrLogin,
    sidecar_log,
    types::{ProviderId, ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
    utils::{
        cryptors::netease::{
            cloudmusic_dll_encode_id, generate_client_sign, generate_deviceid, generate_ntes_nuid,
            generate_wnmcid,
        },
        decrypt_eapi_response, encrypt_eapi, encrypt_weapi, generate_weapi_secret_key,
    },
};

const NETEASE_DOMAIN: &str = "https://music.163.com";
const EAPI_ANONIMOUS_URL: &str = "https://interfacepc.music.163.com/eapi/register/anonimous";
const EAPI_ANONIMOUS_API: &str = "/api/register/anonimous";
const FORM_URLENCODED: &str = "application/x-www-form-urlencoded";
const NETEASE_QR_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0";
const APPVER: &str = "3.1.34.205281";
const CFG: &str = "{\"IuRPVVmc3WWul9fT\":{\"version\":983040,\"appver\":\"3.1.34.205281\"}}";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Safari/537.36 Chrome/91.0.4472.164 NeteaseMusicDesktop/3.1.34.205281";
const OSVER: &str = "Microsoft-Windows-11-Professional-build-114514-64bit";

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
        let key = read_string(body_map(&resp).and_then(|body| body.get("unikey")))
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
        let data = data_map(&resp);
        let img = read_string(data.and_then(|data| data.get("qrimg")))
            .ok_or_else(|| anyhow::anyhow!("NETEASE_QR_IMAGE_MISSING"))?;
        Ok(ProviderLoginQrImage {
            provider: ProviderId::Netease,
            key: normalized_key.to_owned(),
            img,
            url: read_string(data.and_then(|data| data.get("qrurl"))),
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
            let r = {
                let mut result = String::new();

                for _ in 0..5 {
                    match reg().await {
                        Ok((a, b)) => {
                            result = format!("deviceId={a}; clientSign={b};");
                            break;
                        }
                        Err(e) => {
                            sidecar_log::spawn_runtime_log(serde_json::json!(e));
                        }
                    }
                }

                result
            };
            set_runtime_provider_cookie(
                ProviderId::Netease,
                //一定存在的
                format!(
                    "{}; os=pc; WEVNSM=1.0.0; channel=netease; mode=System Product Name; _ntes_nuid={}; WNMCID={}; osver={OSVER}; appver={APPVER}; ",
                    cookie.trim_end_matches(';'),
                    generate_ntes_nuid(),
                    generate_wnmcid()
                )
                //申请成功才持久化
                + &r,
            )
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

/// 匿名注册接口复用同一个 Client，保留连接池避免每次重试重建 TLS 连接
fn anon_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(Client::new)
}

async fn reg() -> Result<(String, String), String> {
    let mut headers = HeaderMap::new();
    let device_id = generate_deviceid();
    let encoded_id = BASE64.encode(format!(
        "{} {}",
        &device_id,
        cloudmusic_dll_encode_id(&device_id),
    ));
    let sign = generate_client_sign(&device_id, "");

    headers.insert(COOKIE, HeaderValue::from_str(&format!("os=pc; deviceId={}; osver={OSVER}; channel=netease; mode=System Product Name; appver=; clientSign={}; MUSIC_SNS=;", &device_id, &sign)).map_err(|e| format!("fail to insert cookie: {}", e.to_string()))?);

    headers.insert(USER_AGENT, HeaderValue::from_static(UA));

    headers.insert(CONTENT_TYPE, HeaderValue::from_static(FORM_URLENCODED));

    headers.insert("mconfig-info", HeaderValue::from_static(CFG));

    let header = serde_json::json!({
        "clientSign": sign,
        "os": "pc",
        "appver": APPVER,
        "requestId": 0,
        "osver": OSVER,
    });

    let body = &serde_json::json!({
        "username": encoded_id,
        "e_r": true,
        "header": serde_json::to_string(&header).map_err(|e| format!("fail to convert Value to String: {}", e.to_string()))?
    });

    let encrypted = encrypt_eapi(EAPI_ANONIMOUS_API, crate::utils::EapiBody::Json(&body))
        .map_err(|e| format!("fail to encrypt req: {}", e.to_string()))?;
    let response = anon_client()
        .post(EAPI_ANONIMOUS_URL)
        .headers(headers)
        .form(&[("params", encrypted.params)])
        .send()
        .await
        .map_err(|e| format!("fail to send req: {}", e.to_string()))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("fail to read resp: {}", e.to_string()))?;
    let decrypted = decrypt_eapi_response(&bytes, false)
        .map_err(|e| format!("fail to decrypt req: {}", e.to_string()))?;
    serde_json::from_slice::<Value>(&decrypted)
        .unwrap()
        .get("code")
        .and_then(|v| v.as_u64())
        .map(|v| v == 200)
        .unwrap_or(false)
        .then_some((device_id, sign))
        .ok_or("failed to reg".into())
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

fn body_map(resp: &NeteaseApiResponse) -> Option<&serde_json::Map<String, Value>> {
    resp.body.as_ref().and_then(Value::as_object)
}

fn data_map(resp: &NeteaseApiResponse) -> Option<&serde_json::Map<String, Value>> {
    body_map(resp)
        .and_then(|body| body.get("data"))
        .and_then(Value::as_object)
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
    let body = body_map(resp);
    let data = data_map(resp);
    read_string(resp.cookie.as_ref())
        .or_else(|| body.and_then(|body| read_string(body.get("cookie"))))
        .or_else(|| data.and_then(|data| read_string(data.get("cookie"))))
        .or_else(|| data.and_then(|data| read_string(data.get("cookies"))))
}

fn read_qr_code(resp: &NeteaseApiResponse) -> i64 {
    let body = body_map(resp);
    let data = data_map(resp);
    read_number(body.and_then(|body| body.get("code")))
        .or_else(|| read_number(data.and_then(|data| data.get("code"))))
        .unwrap_or(0)
}

fn read_qr_message(resp: &NeteaseApiResponse) -> Option<String> {
    let body = body_map(resp);
    let data = data_map(resp);
    read_string(body.and_then(|body| body.get("message")))
        .or_else(|| data.and_then(|data| read_string(data.get("message"))))
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
        let url = format!("{NETEASE_DOMAIN}/login?codekey={key}");
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
    let mut body = match payload {
        Value::Object(map) => map,
        _ => Default::default(),
    };
    body.insert("csrf_token".to_owned(), Value::String(String::new()));
    let encrypted = encrypt_weapi(&Value::Object(body), Some(&generate_weapi_secret_key()))
        .map_err(|err| anyhow::anyhow!("NETEASE_QR_ENCRYPT_FAILED: {err}"))?;

    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(NETEASE_QR_USER_AGENT));
    headers.insert(REFERER, HeaderValue::from_static(NETEASE_DOMAIN));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(FORM_URLENCODED));
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
    let a = generate_ntes_nuid();

    format!(
        "__remember_me=true; _ntes_nuid={a}; _ntes_nnid={a},{timestamp}; WEVNSM=1.0.0; WNMCID={}; appver={APPVER}; channel=netease; os=pc; osver={OSVER}",
        generate_wnmcid()
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
    use crate::utils::cryptors::netease::generate_client_sign;
    use crate::utils::{decrypt_eapi_response, encrypt_eapi};

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

    #[tokio::test]
    async fn t() {
        let mut headers = HeaderMap::new();
        let device_id = generate_deviceid();
        println!("{}", device_id);
        let encoded_id = BASE64.encode(format!(
            "{} {}",
            &device_id,
            cloudmusic_dll_encode_id(&device_id),
        ));
        let sign = generate_client_sign(&device_id, "");
        headers.insert(COOKIE, HeaderValue::from_str(&format!("os=pc; deviceId={}; osver={OSVER}; channel=netease; mode=System Product Name; appver={APPVER}; clientSign={}; MUSIC_SNS=; NMTID={}", &device_id, &sign, "0102030405060708090a0b0c0d0e0f10")).expect("fail to generate cookie"));
        headers.insert(USER_AGENT, HeaderValue::from_static(UA));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        headers.insert("mconfig-info", HeaderValue::from_static(CFG));
        let body = &serde_json::json!({
            "username": encoded_id,
            "e_r": true,
            "header": "{\"clientSign\":\"".to_owned() + &sign + "\",\"os\":\"pc\",\"appver\":\"" + APPVER + "\",\"requestId\":0,\"osver\":\"" + OSVER + "\"}"
        });

        let encrypted = encrypt_eapi(EAPI_ANONIMOUS_API, crate::utils::EapiBody::Json(&body))
            .expect("failed to encrypt params");
        let response = anon_client()
            .post(EAPI_ANONIMOUS_URL)
            .headers(headers)
            .form(&[("params", encrypted.params)])
            .send()
            .await
            .expect("fail to send req");
        let bytes = response.bytes().await.expect("failed to read resp");
        let decrypted = decrypt_eapi_response(&bytes, false).expect("fail to decrypt resp");
        println!(
            "{}",
            serde_json::from_slice::<Value>(&decrypted)
                .unwrap()
                .get("code")
                .expect("failed to read code")
                .to_string()
        );
    }
}
