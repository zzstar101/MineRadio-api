use axum::{
    body::Body,
    http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode},
    response::Response,
};

use crate::http::response::fail;

const PLAYBACK_REQUEST_HEADERS: [&str; 1] = ["range"];
const UPSTREAM_RESPONSE_HEADERS: [&str; 7] = [
    "content-type",
    "content-length",
    "accept-ranges",
    "content-range",
    "cache-control",
    "etag",
    "last-modified",
];

pub struct AudioProxyRequest {
    pub target: String,
    pub request: Request<Body>,
}

#[derive(Clone)]
pub struct AudioProxyDeps {
    pub client: reqwest::Client,
}

impl Default for AudioProxyDeps {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Clone)]
pub struct AudioProxy {
    deps: AudioProxyDeps,
}

impl AudioProxy {
    pub async fn resolve(&self, input: AudioProxyRequest) -> Response {
        proxy_audio(input, &self.deps).await
    }
}

pub fn create_audio_proxy(deps: AudioProxyDeps) -> AudioProxy {
    AudioProxy { deps }
}

async fn proxy_audio(input: AudioProxyRequest, deps: &AudioProxyDeps) -> Response {
    let parsed = match parse_target_url(&input.target) {
        Ok(url) => url,
        Err(message) => return bad_request(message),
    };

    let mut builder = deps.client.get(parsed);
    for header in PLAYBACK_REQUEST_HEADERS {
        if let Some(value) = input.request.headers().get(header) {
            builder = builder.header(header, value.clone());
        }
    }

    let upstream = match builder.send().await {
        Ok(response) => response,
        Err(_) => return upstream_failure("upstream audio request failed"),
    };

    let status = upstream.status();
    if !status.is_success() {
        return upstream_failure(format!(
            "upstream audio request returned {}",
            status.as_u16()
        ));
    }

    let headers = response_headers_from(upstream.headers());
    let bytes = match upstream.bytes().await {
        Ok(bytes) => bytes,
        Err(_) => return upstream_failure("upstream audio request failed"),
    };

    Response::builder()
        .status(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY))
        .body(Body::from(bytes))
        .map(|mut response| {
            *response.headers_mut() = headers;
            response
        })
        .unwrap_or_else(|_| upstream_failure("upstream audio request failed"))
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

fn response_headers_from(upstream: &reqwest::header::HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    for header in UPSTREAM_RESPONSE_HEADERS {
        if let Some(value) = upstream.get(header) {
            if let Ok(name) = HeaderName::from_bytes(header.as_bytes()) {
                headers.insert(name, value.clone());
            }
        }
    }
    headers
}

fn bad_request(message: impl Into<String>) -> Response {
    fail(StatusCode::BAD_REQUEST, "BAD_REQUEST", message)
}

fn upstream_failure(message: impl Into<String>) -> Response {
    fail(StatusCode::BAD_GATEWAY, "UPSTREAM_AUDIO_PROXY", message)
}
