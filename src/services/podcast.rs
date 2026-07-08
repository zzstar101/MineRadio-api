use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type NeteaseResponse = Value;

#[async_trait]
pub trait PodcastRequester: Send + Sync {
    async fn cloudsearch(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_hot(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_detail(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_program(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_sublist(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn user_audio(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn dj_paygift(&self, params: HashMap<String, Value>) -> anyhow::Result<NeteaseResponse>;
    async fn record_recent_voice(
        &self,
        params: HashMap<String, Value>,
    ) -> anyhow::Result<NeteaseResponse>;
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastLoginInfo {
    pub logged_in: bool,
    pub user_id: Option<Value>,
}

#[derive(Default)]
pub struct PodcastServiceDeps {
    pub requester: Option<Box<dyn PodcastRequester>>,
}

#[derive(Default)]
pub struct PodcastService {
    deps: PodcastServiceDeps,
}

impl PodcastService {
    pub async fn search(&self, _params: PodcastSearchParams) -> anyhow::Result<Value> {
        anyhow::bail!("podcast service is not implemented")
    }

    pub async fn hot(&self, _params: PodcastPageParams) -> anyhow::Result<Value> {
        anyhow::bail!("podcast service is not implemented")
    }

    pub async fn detail(&self, _params: PodcastDetailParams) -> anyhow::Result<Value> {
        anyhow::bail!("podcast service is not implemented")
    }

    pub async fn programs(&self, _params: PodcastProgramsParams) -> anyhow::Result<Value> {
        anyhow::bail!("podcast service is not implemented")
    }

    pub async fn my(&self) -> anyhow::Result<Value> {
        anyhow::bail!("podcast service is not implemented")
    }

    pub async fn my_items(&self, _params: PodcastMyItemsParams) -> anyhow::Result<Value> {
        anyhow::bail!("podcast service is not implemented")
    }

    pub async fn dj_beatmap(&self, _params: PodcastBeatmapParams) -> anyhow::Result<Value> {
        anyhow::bail!("podcast service is not implemented")
    }

    pub fn deps(&self) -> &PodcastServiceDeps {
        &self.deps
    }
}

pub fn create_podcast_service(deps: PodcastServiceDeps) -> PodcastService {
    PodcastService { deps }
}

#[derive(Clone, Debug)]
pub struct PodcastSearchParams {
    pub keywords: String,
    pub limit: u32,
}

#[derive(Clone, Debug)]
pub struct PodcastPageParams {
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct PodcastDetailParams {
    pub rid: String,
}

#[derive(Clone, Debug)]
pub struct PodcastProgramsParams {
    pub rid: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct PodcastMyItemsParams {
    pub key: String,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodcastBeatmapParams {
    pub url: String,
    pub duration_sec: u32,
    pub intro_sec: Option<u32>,
}

pub fn map_podcast_radio(_raw: Value) -> Value {
    Value::Null
}

pub fn map_podcast_program(_raw: Value, _fallback_radio: Option<Value>) -> Value {
    Value::Null
}

pub fn podcast_collection_meta(_key: &str, _items: &[Value]) -> Value {
    Value::Null
}
