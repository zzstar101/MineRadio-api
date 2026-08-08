use std::{future::Future, panic::AssertUnwindSafe, sync::Arc};

use futures_util::FutureExt;

use crate::{
    error::{ApiResult, from_provider_error},
    providers::{ProviderAdapter, ProviderResult},
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, RecommendationPage, SongLikeAck,
        SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    },
};

/// A static facade for one music provider.
#[derive(Clone)]
pub struct ProviderApi {
    adapter: Arc<dyn ProviderAdapter>,
}

impl ProviderApi {
    pub(crate) fn new(adapter: Arc<dyn ProviderAdapter>) -> Self {
        Self { adapter }
    }

    pub fn id(&self) -> ProviderId {
        self.adapter.id()
    }

    async fn call<T>(
        &self,
        operation: &'static str,
        future: impl Future<Output = ProviderResult<T>>,
    ) -> ApiResult<T> {
        match AssertUnwindSafe(future).catch_unwind().await {
            Ok(result) => result.map_err(from_provider_error),
            Err(_) => {
                tracing::error!(provider = %self.id(), operation, "provider operation panicked");
                Err(crate::error::ApiError::new(
                    crate::error::ApiErrorCode::Internal,
                    "internal error",
                ))
            }
        }
    }

    pub async fn search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ApiResult<Vec<Track>> {
        self.call(
            "search_track",
            self.adapter.search_track(keyword, offset, limit),
        )
        .await
    }

    pub async fn search_album(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ApiResult<Vec<AlbumSummary>> {
        self.call(
            "search_album",
            self.adapter.search_album(keyword, offset, limit),
        )
        .await
    }

    pub async fn search_playlist(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ApiResult<Vec<PlaylistSummary>> {
        self.call(
            "search_playlist",
            self.adapter.search_playlist(keyword, offset, limit),
        )
        .await
    }

    pub async fn song_url(
        &self,
        track: &Track,
        options: Option<SongUrlOptions>,
    ) -> ApiResult<SongUrlResult> {
        self.call("song_url", self.adapter.song_url(track, options))
            .await
    }

    pub async fn track_qualities(&self, track: &Track) -> ApiResult<TrackQualityAvailability> {
        self.call("track_qualities", self.adapter.track_qualities(track))
            .await
    }

    pub async fn lyric(&self, track: &Track) -> ApiResult<LyricPayload> {
        self.call("lyric", self.adapter.lyric(track)).await
    }

    pub async fn playlist_list(&self) -> ApiResult<Vec<PlaylistSummary>> {
        self.call("playlist_list", self.adapter.playlist_list())
            .await
    }

    pub async fn playlist_detail(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> ApiResult<PlaylistDetail> {
        self.call(
            "playlist_detail",
            self.adapter.playlist_detail(id, offset, limit),
        )
        .await
    }

    pub async fn radio_detail(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> ApiResult<PlaylistDetail> {
        self.call("radio_detail", self.adapter.radio_detail(id, offset, limit))
            .await
    }

    pub async fn login_status(&self) -> ApiResult<ProviderLoginStatus> {
        self.call("login_status", self.adapter.login_status()).await
    }

    pub async fn logout(&self) -> ApiResult<()> {
        self.call("logout", self.adapter.logout()).await
    }

    pub async fn like_song(&self, id: &str, liked: bool) -> ApiResult<SongLikeAck> {
        self.call("like_song", self.adapter.like_song(id, liked))
            .await
    }

    pub async fn check_song_likes(&self, ids: &[String]) -> ApiResult<SongLikeCheckAck> {
        self.call("check_song_likes", self.adapter.check_song_likes(ids))
            .await
    }

    pub async fn update_song_in_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
        adding: bool,
    ) -> ApiResult<PlaylistAddSongAck> {
        self.call(
            "update_song_in_playlist",
            self.adapter
                .update_song_in_playlist(playlist_id, track_id, adding),
        )
        .await
    }

    pub async fn album_list(&self) -> ApiResult<Vec<AlbumSummary>> {
        self.call("album_list", self.adapter.album_list()).await
    }

    pub async fn album_detail(&self, id: &str, offset: u32, limit: u32) -> ApiResult<AlbumDetail> {
        self.call("album_detail", self.adapter.album_detail(id, offset, limit))
            .await
    }

    pub async fn recommendation_page(&self) -> ApiResult<RecommendationPage> {
        self.call("recommendation_page", self.adapter.recommendation_page())
            .await
    }
}
