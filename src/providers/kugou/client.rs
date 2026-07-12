#![allow(dead_code)]

use std::{collections::BTreeMap, time::SystemTime};

use md5::{Digest, Md5};
use reqwest::{Client, Method, Response};
use serde_json::Value;

use crate::providers::{
    ProviderResult,
    error::{ProviderError, ProviderErrorCode},
};

const GATEWAY_URL: &str = "https://gateway.kugou.com";
const APP_ID: &str = "1005";
const CLIENT_VERSION: &str = "20489";
const USER_AGENT: &str = "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi";
const WEB_SIGNATURE_SALT: &str = "NVPh5oo715z5DIWAeQlhMDsWXXQV4hwt";
const ANDROID_SIGNATURE_SALT: &str = "OIlwieks28dk2k092lksi2UIkp";
const REGISTER_SIGNATURE_SALT: &str = "1014";
const SIGN_KEY_SALT: &str = "57ae12eb6890223e355ccfcb74edf70d";

pub type KugouCookie = BTreeMap<String, String>;
pub type KugouParams = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Default)]
pub enum KugouSignature {
    #[default]
    Android,
    Web,
    Register,
}

#[derive(Clone, Debug)]
pub enum KugouRequestBody {
    Json(Value),
    Text(String),
    Bytes(Vec<u8>),
}

impl KugouRequestBody {
    fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Json(value) => value.to_string().into_bytes(),
            Self::Text(value) => value.as_bytes().to_vec(),
            Self::Bytes(value) => value.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct KugouRequest {
    pub method: Method,
    pub path: String,
    pub base_url: Option<String>,
    pub params: KugouParams,
    pub body: Option<KugouRequestBody>,
    pub headers: BTreeMap<String, String>,
    pub signature: KugouSignature,
    pub cookie: KugouCookie,
    pub encrypt_key: bool,
    pub clear_default_params: bool,
    pub skip_signature: bool,
}

impl KugouRequest {
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            base_url: None,
            params: KugouParams::new(),
            body: None,
            headers: BTreeMap::new(),
            signature: KugouSignature::Android,
            cookie: KugouCookie::new(),
            encrypt_key: false,
            clear_default_params: false,
            skip_signature: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct KugouResponse {
    pub body: Value,
    pub cookies: Vec<String>,
    pub ssa_code: Option<String>,
}

#[derive(Clone)]
pub struct KugouClient {
    http: Client,
}

impl KugouClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
        }
    }

    pub fn with_http_client(http: Client) -> Self {
        Self { http }
    }

    pub async fn request(&self, request: KugouRequest) -> ProviderResult<KugouResponse> {
        let clienttime = unix_seconds().to_string();
        let dfid = request
            .cookie
            .get("dfid")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| "-".to_owned());
        let mid = request
            .cookie
            .get("KUGOU_API_MID")
            .cloned()
            .unwrap_or_else(|| "undefined".to_owned());

        let mut params = if request.clear_default_params {
            request.params
        } else {
            let mut params = KugouParams::from([
                ("dfid".to_owned(), Value::String(dfid.clone())),
                ("mid".to_owned(), Value::String(mid.clone())),
                ("uuid".to_owned(), Value::String("-".to_owned())),
                ("appid".to_owned(), Value::String(APP_ID.to_owned())),
                (
                    "clientver".to_owned(),
                    Value::String(CLIENT_VERSION.to_owned()),
                ),
                ("clienttime".to_owned(), Value::String(clienttime.clone())),
            ]);
            if let Some(token) = request
                .cookie
                .get("token")
                .filter(|value| !value.is_empty())
            {
                params.insert("token".to_owned(), Value::String(token.clone()));
            }
            if let Some(userid) = request.cookie.get("userid").filter(|value| *value != "0") {
                params.insert("userid".to_owned(), Value::String(userid.clone()));
            }
            params.extend(request.params);
            params
        };

        if request.encrypt_key {
            let hash = params
                .get("hash")
                .map(json_value_to_string)
                .unwrap_or_else(|| "undefined".to_owned());
            let userid = params
                .get("userid")
                .map(json_value_to_string)
                .unwrap_or_else(|| "0".to_owned());
            let appid = params
                .get("appid")
                .map(json_value_to_string)
                .unwrap_or_else(|| APP_ID.to_owned());
            params.insert(
                "key".to_owned(),
                Value::String(sign_key(&hash, &mid, &userid, &appid)),
            );
        }

        let body = request.body.as_ref().map(KugouRequestBody::bytes);
        if !request.skip_signature && !params.contains_key("signature") {
            let signature = match request.signature {
                KugouSignature::Android => signature_android(&params, body.as_deref()),
                KugouSignature::Web => signature_web(&params),
                KugouSignature::Register => signature_register(&params),
            };
            params.insert("signature".to_owned(), Value::String(signature));
        }

        let url = format!(
            "{}/{}",
            request
                .base_url
                .as_deref()
                .unwrap_or(GATEWAY_URL)
                .trim_end_matches('/'),
            request.path.trim_start_matches('/')
        );
        let query = params
            .iter()
            .map(|(key, value)| (key.as_str(), json_value_to_string(value)))
            .collect::<Vec<_>>();
        let mut request_builder = self.http.request(request.method, url).query(&query).header(
            "user-agent",
            request
                .headers
                .get("User-Agent")
                .cloned()
                .unwrap_or_else(|| USER_AGENT.to_owned()),
        );
        for (name, value) in request.headers {
            if !name.eq_ignore_ascii_case("user-agent") {
                request_builder = request_builder.header(name, value);
            }
        }
        request_builder = request_builder
            .header("dfid", dfid)
            .header("clienttime", clienttime)
            .header("mid", mid)
            .header("kg-rc", "1")
            .header("kg-thash", "5d816a0")
            .header("kg-rec", "1")
            .header("kg-rf", "B9EDA08A64250DEFFBCADDEE00F8F25F");
        if !request.cookie.is_empty() {
            request_builder = request_builder.header("cookie", cookie_header(&request.cookie));
        }
        if let Some(body) = body {
            request_builder = request_builder.body(body);
        }

        let response = request_builder
            .send()
            .await
            .map_err(|error| unavailable_error(error.to_string()))?;
        parse_response(response).await
    }
}

impl Default for KugouClient {
    fn default() -> Self {
        Self::new()
    }
}

pub fn signature_web(params: &KugouParams) -> String {
    let params = signature_pairs(params).join("");
    md5_hex(format!("{WEB_SIGNATURE_SALT}{params}{WEB_SIGNATURE_SALT}").as_bytes())
}

pub fn signature_android(params: &KugouParams, body: Option<&[u8]>) -> String {
    let params = signature_pairs(params).join("");
    let mut hasher = Md5::new();
    hasher.update(ANDROID_SIGNATURE_SALT.as_bytes());
    hasher.update(params.as_bytes());
    if let Some(body) = body {
        hasher.update(body);
    }
    hasher.update(ANDROID_SIGNATURE_SALT.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn signature_register(params: &KugouParams) -> String {
    let mut values = params
        .values()
        .map(json_value_to_string)
        .collect::<Vec<_>>();
    values.sort();
    md5_hex(
        format!(
            "{REGISTER_SIGNATURE_SALT}{}{REGISTER_SIGNATURE_SALT}",
            values.join("")
        )
        .as_bytes(),
    )
}

pub fn sign_key(hash: &str, mid: &str, userid: &str, appid: &str) -> String {
    md5_hex(format!("{hash}{SIGN_KEY_SALT}{appid}{mid}{userid}").as_bytes())
}

async fn parse_response(response: Response) -> ProviderResult<KugouResponse> {
    let http_status = response.status();
    let cookies = response_cookies(&response);
    let ssa_code = response
        .headers()
        .get("ssa-code")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let text = response
        .text()
        .await
        .map_err(|error| unavailable_error(error.to_string()))?;
    let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
    let api_failed = body.get("status").and_then(Value::as_i64) == Some(0)
        || body
            .get("error_code")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0);
    if !http_status.is_success() || api_failed {
        return Err(ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: "kugou".to_owned(),
            message: body
                .get("msg")
                .or_else(|| body.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("kugou upstream error")
                .to_owned(),
            retryable: true,
            action: None,
            raw_message: Some(body.to_string()),
        });
    }
    Ok(KugouResponse {
        body,
        cookies,
        ssa_code,
    })
}

fn signature_pairs(params: &KugouParams) -> Vec<String> {
    params
        .iter()
        .map(|(key, value)| format!("{key}={}", json_value_to_string(value)))
        .collect()
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn response_cookies(response: &Response) -> Vec<String> {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|header| header.to_str().ok())
        .filter_map(|header| header.split(';').next())
        .map(str::trim)
        .filter(|cookie| !cookie.is_empty() && cookie.contains('='))
        .map(ToOwned::to_owned)
        .collect()
}

fn cookie_header(cookie: &KugouCookie) -> String {
    cookie
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn md5_hex(value: &[u8]) -> String {
    format!("{:x}", Md5::digest(value))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn unavailable_error(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: "kugou".to_owned(),
        message: message.clone(),
        retryable: true,
        action: None,
        raw_message: Some(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_sort_parameters_like_the_js_client() {
        let params = KugouParams::from([
            ("b".to_owned(), Value::String("2".to_owned())),
            ("a".to_owned(), Value::String("1".to_owned())),
        ]);

        assert_eq!(signature_web(&params), "70ccbef64fdcc9271fe883d1d7f07395");
        assert_eq!(
            signature_android(&params, Some(br#"{"name":"test"}"#)),
            "f3e569d8863a00ed4bac93d6897dba77"
        );
        assert_eq!(
            signature_register(&params),
            "3be0f2ebde7da28161927749ab76ba88"
        );
    }

    #[test]
    fn sign_key_matches_the_android_algorithm() {
        assert_eq!(
            sign_key("hash", "mid", "42", "1005"),
            "d467a74e2b00b07c297161131cfd5db4"
        );
    }
}
