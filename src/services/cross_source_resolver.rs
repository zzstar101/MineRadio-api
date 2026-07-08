use std::collections::HashMap;

use crate::{
    providers::ProviderAdapter,
    types::{ProviderId, SongUrlOptions, SongUrlResult, Track},
};

pub type ProviderMap = HashMap<ProviderId, Box<dyn ProviderAdapter>>;

#[derive(Default)]
pub struct CrossSourceResolverDeps {
    pub providers: Option<ProviderMap>,
    pub provider_order: Option<Vec<ProviderId>>,
}

pub struct ResolveSearchQuery {
    pub keyword: String,
    pub provider: Option<ProviderId>,
    pub limit: u32,
}

#[derive(Default)]
pub struct CrossSourceResolver {
    deps: CrossSourceResolverDeps,
}

impl CrossSourceResolver {
    pub async fn resolve_search(&self, _query: ResolveSearchQuery) -> anyhow::Result<Vec<Track>> {
        anyhow::bail!("cross-source resolver is not implemented")
    }

    pub async fn resolve_song_url(
        &self,
        _track: Track,
        _opts: Option<SongUrlOptions>,
    ) -> anyhow::Result<SongUrlResult> {
        anyhow::bail!("cross-source resolver is not implemented")
    }

    pub fn deps(&self) -> &CrossSourceResolverDeps {
        &self.deps
    }
}

pub fn create_cross_source_resolver(deps: CrossSourceResolverDeps) -> CrossSourceResolver {
    CrossSourceResolver { deps }
}
