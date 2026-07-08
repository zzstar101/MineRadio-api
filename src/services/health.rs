use serde::Serialize;

use crate::{
    config::Config,
    providers::registry::{CapabilityMatrix, PROVIDER_IDS, build_capability_matrix},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthPayload {
    pub ok: bool,
    pub app_version: String,
    pub api_version: String,
    pub schema_version: String,
    pub providers: Vec<&'static str>,
    pub provider_status: CapabilityMatrix,
}

pub fn snapshot(config: &Config) -> HealthPayload {
    HealthPayload {
        ok: true,
        app_version: config.app_version.clone(),
        api_version: config.api_version.clone(),
        schema_version: config.schema_version.clone(),
        providers: PROVIDER_IDS.to_vec(),
        provider_status: build_capability_matrix(),
    }
}
