use std::{collections::HashMap, sync::Arc};

use serde::Serialize;

use crate::types::ProviderId;

use super::ProviderAdapter;

#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderCapability {
    pub id: ProviderId,
    pub search: bool,
    pub song_url: bool,
    pub lyric: bool,
    pub playlists: bool,
    pub login: bool,
}

impl ProviderRegistry {
    pub fn register(&mut self, provider: Arc<dyn ProviderAdapter>) {
        self.providers.insert(provider.id(), provider);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.providers.get(id).cloned()
    }

    pub fn build_capability_matrix(&self) -> Vec<ProviderCapability> {
        self.providers
            .keys()
            .map(|id| ProviderCapability {
                id: id.clone(),
                search: true,
                song_url: true,
                lyric: true,
                playlists: true,
                login: true,
            })
            .collect()
    }
}
