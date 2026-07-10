use serde::Serialize;
use thiserror::Error;

use crate::types::ProviderId;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ProviderErrorCode {
    NoResult,
    NoUrl,
    NoPlaylist,
    LoginRequired,
    Unavailable,
    CopyrightUnavailable,
    PaidRequired,
    TrialOnly,
    VipRequired,
    NotImplemented,
    Internal,
}

#[derive(Clone, Debug, Error, Serialize)]
#[error("{provider}: {message}")]
pub struct ProviderError {
    pub code: ProviderErrorCode,
    pub provider: ProviderId,
    pub message: String,
    pub retryable: bool,
    pub action: Option<String>,
    pub raw_message: Option<String>,
}

impl ProviderError {
    pub fn not_implemented(provider: ProviderId, action: impl Into<String>) -> Self {
        let action = action.into();

        Self {
            code: ProviderErrorCode::NotImplemented,
            provider,
            message: format!("{action} is not implemented"),
            retryable: false,
            action: Some(action),
            raw_message: None,
        }
    }
}
