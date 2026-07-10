use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::Response,
};

use crate::http::response::fail;

const UPSTREAM_RESPONSE_HEADERS: [&str; 5] = [
    "content-type",
    "content-length",
    "cache-control",
    "etag",
    "last-modified",
];
const COVER_PROXY_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

pub struct ImageProxyRequest {
    pub target: String,
}

#[derive(Clone)]
pub struct ImageProxyDeps {
    pub client: reqwest::Client,
}

impl Default for ImageProxyDeps {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Clone)]
pub struct ImageProxy {
    deps: ImageProxyDeps,
}

impl ImageProxy {
    pub async fn resolve(&self, input: ImageProxyRequest) -> Response {
        proxy_image(input, &self.deps).await
    }
}

pub fn create_image_proxy(deps: ImageProxyDeps) -> ImageProxy {
    ImageProxy { deps }
}

async fn proxy_image(input: ImageProxyRequest, deps: &ImageProxyDeps) -> Response {
    let parsed = match parse_target_url(&input.target) {
        Ok(url) => url,
        Err(message) => return bad_request(message),
    };

    let upstream = match deps
        .client
        .get(parsed.clone())
        .header("user-agent", COVER_PROXY_USER_AGENT)
        .header("referer", referer_for_cover_url(parsed.as_str()))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return upstream_failure("upstream image request failed"),
    };

    let status = upstream.status();
    if !status.is_success() {
        return upstream_failure(format!(
            "upstream image request returned {}",
            status.as_u16()
        ));
    }
    if !is_image_response(upstream.headers()) {
        return upstream_failure("upstream image request returned non-image content");
    }

    let headers = response_headers_from(upstream.headers());
    let bytes = match upstream.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => return upstream_failure("upstream image request failed"),
    };

    Response::builder()
        .status(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY))
        .body(Body::from(bytes))
        .map(|mut response| {
            *response.headers_mut() = headers;
            response
        })
        .unwrap_or_else(|_| upstream_failure("upstream image request failed"))
}

fn parse_target_url(target: &str) -> Result<url::Url, &'static str> {
    if target.trim().is_empty() {
        return Err("url required");
    }
    let url = url::Url::parse(target).map_err(|_| "invalid url")?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err("url must use http or https"),
    }
}

fn is_image_response(upstream: &reqwest::header::HeaderMap) -> bool {
    upstream
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_lowercase().starts_with("image/"))
        .unwrap_or(false)
}

fn response_headers_from(upstream: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("cross-origin"),
    );
    for header in UPSTREAM_RESPONSE_HEADERS {
        if let Some(value) = upstream.get(header) {
            if let Ok(name) = HeaderName::from_bytes(header.as_bytes()) {
                headers.insert(name, value.clone());
            }
        }
    }
    headers
}

fn referer_for_cover_url(target: &str) -> &'static str {
    if let Ok(url) = url::Url::parse(target) {
        let host = url.host_str().unwrap_or_default().to_lowercase();
        if host.contains("qq.com") || host.contains("qpic.cn") || host.contains("gtimg.cn") {
            return "https://y.qq.com/";
        }
    }
    "https://music.163.com/"
}

fn bad_request(message: impl Into<String>) -> Response {
    fail(StatusCode::BAD_REQUEST, "BAD_REQUEST", message)
}

fn upstream_failure(message: impl Into<String>) -> Response {
    fail(StatusCode::BAD_GATEWAY, "UPSTREAM_IMAGE_PROXY", message)
}
