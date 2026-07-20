use std::{collections::HashMap, sync::Arc};

use serde::Serialize;

use crate::types::ProviderId;

use super::ProviderAdapter;

pub const PROVIDER_IDS: [ProviderId; 3] = [ProviderId::Netease, ProviderId::Qq, ProviderId::Soda];

const NETEASE_CAPABILITIES: [&str; 9] = [
    "search",
    "songUrl",
    "lyric",
    "playlistList",
    "playlistDetail",
    "loginStatus",
    "logout",
    "like",
    "quality",
];

const QQ_CAPABILITIES: [&str; 8] = [
    "search",
    "songUrl",
    "lyric",
    "playlistList",
    "playlistDetail",
    "loginStatus",
    "logout",
    "quality",
];

const SODA_CAPABILITIES: [&str; 9] = [
    "search",
    "songUrl",
    "lyric",
    "playlistList",
    "playlistDetail",
    "loginStatus",
    "logout",
    "like",
    "quality",
];

#[derive(Default)]
pub struct ProviderRegistry {
    providers: HashMap<ProviderId, Arc<dyn ProviderAdapter>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CapabilityMatrix {
    pub version: &'static str,
    pub providers: Vec<ProviderStatusEntry>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatusEntry {
    pub provider_id: ProviderId,
    pub available: bool,
    pub capabilities: Vec<&'static str>,
    pub message: &'static str,
}

impl ProviderRegistry {
    pub fn register(&mut self, provider: Arc<dyn ProviderAdapter>) {
        self.providers.insert(provider.id(), provider);
    }

    pub fn get(&self, id: &ProviderId) -> Option<Arc<dyn ProviderAdapter>> {
        self.providers.get(id).cloned()
    }

    pub fn all(&self) -> HashMap<ProviderId, Arc<dyn ProviderAdapter>> {
        self.providers.clone()
    }

    pub fn build_capability_matrix(&self) -> CapabilityMatrix {
        build_capability_matrix()
    }
}

pub fn build_capability_matrix() -> CapabilityMatrix {
    CapabilityMatrix {
        version: "0.1.0",
        providers: vec![
            ProviderStatusEntry {
                provider_id: ProviderId::Netease,
                available: true,
                capabilities: NETEASE_CAPABILITIES.to_vec(),
                message: "online",
            },
            ProviderStatusEntry {
                provider_id: ProviderId::Qq,
                available: true,
                capabilities: QQ_CAPABILITIES.to_vec(),
                message: "online",
            },
            ProviderStatusEntry {
                provider_id: ProviderId::Soda,
                available: true,
                capabilities: SODA_CAPABILITIES.to_vec(),
                message: "online",
            },
        ],
    }
}
