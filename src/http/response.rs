use axum::{
    Json,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiSuccess<T: Serialize> {
    pub ok: bool,
    pub data: T,
}

#[derive(Debug, Serialize)]
pub struct ApiFailure {
    pub ok: bool,
    pub error: ApiError,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

pub fn ok<T: Serialize>(data: T) -> Response {
    json(ApiSuccess { ok: true, data }, StatusCode::OK)
}

pub fn json<T: Serialize>(body: T, status: StatusCode) -> Response {
    (status, sidecar_cors_headers(), Json(body)).into_response()
}

pub fn fail(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Response {
    json(
        ApiFailure {
            ok: false,
            error: ApiError {
                code: code.into(),
                message: message.into(),
            },
        },
        status,
    )
}

pub fn cors_preflight() -> Response {
    (StatusCode::NO_CONTENT, sidecar_cors_headers()).into_response()
}

fn sidecar_cors_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET,POST,DELETE,OPTIONS"),
    );
    headers.insert(
        "access-control-allow-headers",
        HeaderValue::from_static("content-type,authorization,range"),
    );
    headers.insert(
        "access-control-expose-headers",
        HeaderValue::from_static("content-length,content-range,accept-ranges,content-type"),
    );
    headers.insert("access-control-max-age", HeaderValue::from_static("86400"));
    headers
}
