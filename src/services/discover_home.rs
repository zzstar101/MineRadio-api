use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::{providers::ProviderAdapter, types::ProviderId};

pub type NeteaseResponse = Value;
pub type DiscoverRequestParams = HashMap<String, Value>;

#[async_trait]
pub trait DiscoverRequester: Send + Sync {
    async fn personalized(&self, params: DiscoverRequestParams) -> anyhow::Result<NeteaseResponse>;
    async fn dj_hot(&self, params: DiscoverRequestParams) -> anyhow::Result<NeteaseResponse>;
    async fn recommend_resource(
        &self,
        params: DiscoverRequestParams,
    ) -> anyhow::Result<NeteaseResponse>;
    async fn recommend_songs(
        &self,
        params: DiscoverRequestParams,
    ) -> anyhow::Result<NeteaseResponse>;
}

pub struct DiscoverHomeServiceOptions {
    pub provider_adapters: HashMap<ProviderId, Box<dyn ProviderAdapter>>,
    pub discover_requester: Option<Box<dyn DiscoverRequester>>,
}

pub async fn build_discover_home(_options: DiscoverHomeServiceOptions) -> anyhow::Result<Value> {
    anyhow::bail!("discover home service is not implemented")
}
