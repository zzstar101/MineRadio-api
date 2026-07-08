use async_trait::async_trait;
use serde_json::Value;

use crate::{
    services::auth_session::set_runtime_provider_cookie,
    types::{ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
};

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

impl NeteaseQrLoginService {
    pub async fn create_key(&self) -> anyhow::Result<ProviderLoginQrKey> {
        let resp = self
            .deps
            .qr_key
            .call(serde_json::json!({ "timestamp": self.now() }))
            .await?;
        let key = read_string(response_data(&resp).get("unikey"))
            .ok_or_else(|| anyhow::anyhow!("NETEASE_QR_KEY_MISSING"))?;
        Ok(ProviderLoginQrKey {
            provider: "netease".to_owned(),
            key,
        })
    }

    pub async fn create_image(&self, key: &str) -> anyhow::Result<ProviderLoginQrImage> {
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
            provider: "netease".to_owned(),
            key: normalized_key.to_owned(),
            img,
            url: read_string(data.get("qrurl")),
        })
    }

    pub async fn check(&self, key: &str) -> anyhow::Result<ProviderLoginQrCheck> {
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
            set_runtime_provider_cookie("netease".to_owned(), cookie)
                .await
                .map_err(|err| anyhow::anyhow!(err))?;
        }

        Ok(ProviderLoginQrCheck {
            provider: "netease".to_owned(),
            key: normalized_key.to_owned(),
            code,
            message: read_qr_message(&resp),
            logged_in: stored,
            scanned: Some(code == 802),
            expired: Some(code == 800),
            stored: Some(stored),
        })
    }

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
