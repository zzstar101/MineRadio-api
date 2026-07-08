use axum::{
    Json,
    http::StatusCode,
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

pub fn ok<T: Serialize>(data: T) -> Json<ApiSuccess<T>> {
    Json(ApiSuccess { ok: true, data })
}

pub fn fail(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Response {
    (
        status,
        Json(ApiFailure {
            ok: false,
            error: ApiError {
                code: code.into(),
                message: message.into(),
            },
        }),
    )
        .into_response()
}
