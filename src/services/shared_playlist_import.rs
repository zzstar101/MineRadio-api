use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{providers::ProviderAdapter, types::ProviderId};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SharedPlaylistCandidate {
    pub kind: String,
    pub url: String,
}

pub struct SharedPlaylistImporterDeps {
    pub provider_adapters: HashMap<ProviderId, Box<dyn ProviderAdapter>>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct SharedPlaylistImportError {
    pub code: String,
    pub message: String,
}

pub async fn import_shared_playlist(
    _input: Value,
    _deps: SharedPlaylistImporterDeps,
) -> anyhow::Result<Value> {
    anyhow::bail!("shared playlist import service is not implemented")
}

pub fn detect_shared_playlist(_input: Value) -> Option<SharedPlaylistCandidate> {
    None
}
