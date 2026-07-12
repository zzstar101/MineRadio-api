pub mod error;
pub mod kugou;
pub mod netease;
pub mod qq;
pub mod registry;
pub mod soda;

use async_trait::async_trait;

use crate::types::{
    LyricPayload, PlaylistAddSongAck, PlaylistDetail, PlaylistSummary, ProviderId,
    ProviderLoginStatus, SongLikeAck, SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track,
    TrackQualityAvailability,
};

pub type ProviderResult<T> = std::result::Result<T, error::ProviderError>;

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> ProviderId;

    async fn search(&self, keyword: &str, limit: u32) -> ProviderResult<Vec<Track>>;
    async fn song_url(
        &self,
        track: &Track,
        opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult>;
    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability>;
    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload>;
    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>>;
    async fn playlist_detail(&self, id: &str) -> ProviderResult<PlaylistDetail>;
    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus>;
    async fn logout(&self) -> ProviderResult<()>;

    async fn like_song(&self, _id: &str, _liked: bool) -> ProviderResult<SongLikeAck> {
        Err(error::ProviderError::not_implemented(self.id(), "like"))
    }

    async fn check_song_likes(&self, _ids: &[String]) -> ProviderResult<SongLikeCheckAck> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "check_likes",
        ))
    }

    async fn add_song_to_playlist(
        &self,
        _playlist_id: &str,
        _track_id: &str,
    ) -> ProviderResult<PlaylistAddSongAck> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "add_to_playlist",
        ))
    }
}
