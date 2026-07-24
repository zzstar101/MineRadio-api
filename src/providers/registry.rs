use std::{collections::HashMap, sync::Arc};

use serde::Serialize;

use crate::types::ProviderId;

use super::ProviderAdapter;

pub const PROVIDER_IDS: [ProviderId; 4] = [
    ProviderId::Netease,
    ProviderId::Qq,
    ProviderId::Soda,
    ProviderId::Kugou,
];

const NETEASE_CAPABILITIES: [&str; 15] = [
    "qrLogin",
    "search",
    "songUrl",
    "quality",
    "lyric",
    "playlistList",
    "playlistDetail",
    "loginStatus",
    "logout",
    "like",
    "likeCheck",
    "addToPlaylist",
    "albumList",
    "albumDetail",
    "register",
];

const QQ_CAPABILITIES: [&str; 12] = [
    "qrLogin",
    "search",
    "songUrl",
    "quality",
    "lyric",
    "playlistList",
    "playlistDetail",
    "loginStatus",
    "logout",
    "addToPlaylist",
    "albumList",
    "albumDetail",
];

const SODA_CAPABILITIES: [&str; 13] = [
    "qrLogin",
    "search",
    "songUrl",
    "quality",
    "lyric",
    "playlistList",
    "playlistDetail",
    "loginStatus",
    "logout",
    "like",
    "likeCheck",
    "albumList",
    "albumDetail",
];

const KUGOU_CAPABILITIES: [&str; 5] = ["search", "songUrl", "quality", "lyric", "register"];

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
        let entries: Vec<ProviderStatusEntry> = PROVIDER_IDS
            .iter()
            .map(|id| {
                let available = self.providers.contains_key(id);
                ProviderStatusEntry {
                    provider_id: *id,
                    available,
                    capabilities: capabilities_of(*id).to_vec(),
                    message: if available { "online" } else { "offline" },
                }
            })
            .collect();
        CapabilityMatrix {
            version: "0.2.0",
            providers: entries,
        }
    }
}

fn capabilities_of(id: ProviderId) -> &'static [&'static str] {
    match id {
        ProviderId::Netease => &NETEASE_CAPABILITIES,
        ProviderId::Qq => &QQ_CAPABILITIES,
        ProviderId::Soda => &SODA_CAPABILITIES,
        ProviderId::Kugou => &KUGOU_CAPABILITIES,
        ProviderId::Unknown => &[],
    }
}

pub fn build_capability_matrix() -> CapabilityMatrix {
    CapabilityMatrix {
        version: "0.2.0",
        providers: PROVIDER_IDS
            .iter()
            .map(|id| ProviderStatusEntry {
                provider_id: *id,
                available: true,
                capabilities: capabilities_of(*id).to_vec(),
                message: "online",
            })
            .collect(),
    }
}
