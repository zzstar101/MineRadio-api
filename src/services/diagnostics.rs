use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use serde_json::Value;

use crate::{
    config::LibraryConfig,
    providers::registry::{ProviderRegistry, ProviderStatusEntry},
    services::sidecar_log::{redact_log_value, sidecar_log_file},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsLogPointers {
    pub sidecar_runtime_log: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsPayload {
    pub ok: bool,
    pub app_version: String,
    pub api_version: String,
    pub schema_version: String,
    pub providers: Vec<ProviderStatusEntry>,
    pub recent_errors: Vec<Value>,
    pub log_pointers: DiagnosticsLogPointers,
}

static RECENT_ERRORS: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();
const RECENT_ERRORS_MAX: usize = 20;

pub fn build_diagnostics(config: &LibraryConfig, providers: &ProviderRegistry) -> DiagnosticsPayload {
    let matrix = providers.build_capability_matrix();
    DiagnosticsPayload {
        ok: true,
        app_version: config.app_version.clone(),
        api_version: config.api_version.clone(),
        schema_version: config.schema_version.clone(),
        providers: matrix.providers,
        recent_errors: recent_errors()
            .lock()
            .map(|errors| errors.iter().map(redact_log_value).collect())
            .unwrap_or_default(),
        log_pointers: DiagnosticsLogPointers {
            sidecar_runtime_log: sanitize_log_pointer(sidecar_log_file()),
        },
    }
}

pub fn snapshot(config: &LibraryConfig, providers: &ProviderRegistry) -> DiagnosticsPayload {
    build_diagnostics(config, providers)
}

pub fn push_recent_error(entry: Value) {
    if let Ok(mut errors) = recent_errors().lock() {
        errors.push(entry);
        if errors.len() > RECENT_ERRORS_MAX {
            errors.remove(0);
        }
    }
}

fn recent_errors() -> &'static Mutex<Vec<Value>> {
    RECENT_ERRORS.get_or_init(|| Mutex::new(Vec::new()))
}

fn sanitize_log_pointer(pointer: Option<String>) -> Option<String> {
    pointer.and_then(|value| match redact_log_value(&Value::String(value)) {
        Value::String(redacted) => Some(redacted),
        _ => None,
    })
}
