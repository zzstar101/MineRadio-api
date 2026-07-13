#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    providers::{ProviderAdapter, ProviderResult, error::ProviderError},
    types::{
        LyricPayload, PlaylistDetail, PlaylistSummary, ProviderId, ProviderLoginStatus,
        SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    },
};

use super::client::KugouClient;

#[derive(Clone, Default)]
pub struct KugouAdapter {
    client: Arc<KugouClient>,
}

impl KugouAdapter {
    pub fn new(client: Arc<KugouClient>) -> Self {
        Self { client }
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new(Arc::new(KugouClient::new())))
    }
}

#[async_trait]
impl ProviderAdapter for KugouAdapter {
    fn id(&self) -> ProviderId {
        "kugou".to_owned()
    }

    async fn search(&self, _keyword: &str, _limit: u32) -> ProviderResult<Vec<Track>> {
        Err(not_implemented("search"))
    }

    async fn song_url(
        &self,
        _track: &Track,
        _opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult> {
        Err(not_implemented("song_url"))
    }

    async fn track_qualities(&self, _track: &Track) -> ProviderResult<TrackQualityAvailability> {
        Err(not_implemented("track_qualities"))
    }

    async fn lyric(&self, _track: &Track) -> ProviderResult<LyricPayload> {
        Err(not_implemented("lyric"))
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        Err(not_implemented("playlist_list"))
    }

    async fn playlist_detail(&self, _id: &str) -> ProviderResult<PlaylistDetail> {
        Err(not_implemented("playlist_detail"))
    }

    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
        Err(not_implemented("login_status"))
    }

    async fn logout(&self) -> ProviderResult<()> {
        Err(not_implemented("logout"))
    }
}

fn not_implemented(action: &str) -> ProviderError {
    ProviderError::not_implemented("kugou".to_owned(), action)
}
