//! Public error contract for MineRadio API consumers.
//!
//! Error codes describe stable, caller-visible failure semantics. They must
//! not encode an HTTP route, provider implementation detail, or local storage
//! mechanism. Internal provider errors are mapped to this contract later at
//! the application boundary.

use std::fmt;

use serde::Serialize;

/// Stable error codes returned by public API methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApiErrorCode {
    BadRequest,
    NotFound,
    LoginRequired,
    Unavailable,
    CopyrightUnavailable,
    PaidRequired,
    TrialOnly,
    VipRequired,
    InvalidResponse,
    NotImplemented,
    Internal,
}

impl ApiErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "BAD_REQUEST",
            Self::NotFound => "NOT_FOUND",
            Self::LoginRequired => "LOGIN_REQUIRED",
            Self::Unavailable => "UNAVAILABLE",
            Self::CopyrightUnavailable => "COPYRIGHT_UNAVAILABLE",
            Self::PaidRequired => "PAID_REQUIRED",
            Self::TrialOnly => "TRIAL_ONLY",
            Self::VipRequired => "VIP_REQUIRED",
            Self::InvalidResponse => "INVALID_RESPONSE",
            Self::NotImplemented => "NOT_IMPLEMENTED",
            Self::Internal => "INTERNAL",
        }
    }
}

impl fmt::Display for ApiErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned by every public API method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiError {
    pub code: ApiErrorCode,
    pub message: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

pub(crate) fn from_provider_error(err: crate::providers::error::ProviderError) -> ApiError {
    use crate::providers::error::ProviderErrorCode;

    let code = match err.code {
        ProviderErrorCode::InvalidResponse => ApiErrorCode::InvalidResponse,
        ProviderErrorCode::NoResult | ProviderErrorCode::NoPlaylist => ApiErrorCode::NotFound,
        ProviderErrorCode::NoUrl | ProviderErrorCode::Unavailable => ApiErrorCode::Unavailable,
        ProviderErrorCode::LoginRequired => ApiErrorCode::LoginRequired,
        ProviderErrorCode::CopyrightUnavailable => ApiErrorCode::CopyrightUnavailable,
        ProviderErrorCode::PaidRequired => ApiErrorCode::PaidRequired,
        ProviderErrorCode::TrialOnly => ApiErrorCode::TrialOnly,
        ProviderErrorCode::VipRequired => ApiErrorCode::VipRequired,
        ProviderErrorCode::NotImplemented => ApiErrorCode::NotImplemented,
        ProviderErrorCode::Internal => ApiErrorCode::Internal,
    };

    ApiError::new(code, err.message)
}
