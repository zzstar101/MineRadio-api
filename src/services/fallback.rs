use serde::Serialize;

use crate::{providers::error::ProviderError, types::ProviderId};

const REDACTED_PROVIDER_ERROR_MESSAGE: &str = "provider error redacted";
const SENSITIVE_AUTH_PATTERNS: [&str; 15] = [
    "music_u",
    "__csrf",
    "nmtid",
    "qm_keyst",
    "qqmusic_key",
    "music_key",
    "wxskey",
    "p_skey",
    "skey",
    "psrf_qqaccess_token",
    "psrf_qqrefresh_token",
    "wxrefresh_token",
    "authorization:",
    "cookie:",
    "set-cookie:",
];

#[derive(Debug, Serialize)]
pub struct NormalizedApiResponse {
    pub ok: bool,
    pub error: NormalizedApiError,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedApiError {
    pub code: String,
    pub message: String,
    pub provider: ProviderId,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_message: Option<String>,
}

pub fn redact_error_message(message: impl AsRef<str>) -> String {
    let text = message.as_ref().to_owned();
    let lower = text.to_lowercase();
    if SENSITIVE_AUTH_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
        || lower.contains("bearer ")
    {
        REDACTED_PROVIDER_ERROR_MESSAGE.to_owned()
    } else {
        text
    }
}

pub fn normalize_error(provider: ProviderId, err: &ProviderError) -> NormalizedApiResponse {
    NormalizedApiResponse {
        ok: false,
        error: NormalizedApiError {
            code: format!("{:?}", err.code),
            message: redact_error_message(&err.message),
            provider: err.provider.clone().if_empty(provider),
            retryable: err.retryable,
            action: err.action.clone(),
            raw_message: err.raw_message.as_ref().map(redact_error_message),
        },
    }
}

trait IfEmpty {
    fn if_empty(self, fallback: String) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: String) -> String {
        if self.is_empty() { fallback } else { self }
    }
}
