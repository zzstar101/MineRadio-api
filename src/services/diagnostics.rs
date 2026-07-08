use serde::Serialize;

use crate::server::AppState;

#[derive(Debug, Serialize)]
pub struct DiagnosticsSnapshot {
    pub app_version: String,
    pub api_version: String,
    pub provider_count: usize,
    pub uptime_ms: u128,
}

pub fn snapshot(state: &AppState) -> DiagnosticsSnapshot {
    DiagnosticsSnapshot {
        app_version: state.config.app_version.clone(),
        api_version: state.config.api_version.clone(),
        provider_count: state.providers.build_capability_matrix().len(),
        uptime_ms: state
            .started_at
            .elapsed()
            .map(|d| d.as_millis())
            .unwrap_or(0),
    }
}
