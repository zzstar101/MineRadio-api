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

pub type Result<T> = std::result::Result<T, error::ProviderError>;

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> ProviderId;

    async fn search(&self, keyword: &str, limit: u32) -> Result<Vec<Track>>;
    async fn song_url(&self, track: &Track, opts: Option<SongUrlOptions>) -> Result<SongUrlResult>;
    async fn track_qualities(&self, track: &Track) -> Result<TrackQualityAvailability>;
    async fn lyric(&self, track: &Track) -> Result<LyricPayload>;
    async fn playlist_list(&self) -> Result<Vec<PlaylistSummary>>;
    async fn playlist_detail(&self, id: &str) -> Result<PlaylistDetail>;
    async fn login_status(&self) -> Result<ProviderLoginStatus>;
    async fn logout(&self) -> Result<()>;

    async fn like_song(&self, _id: &str, _liked: bool) -> Result<SongLikeAck> {
        Err(error::ProviderError::not_implemented(self.id(), "like"))
    }

    async fn check_song_likes(&self, _ids: &[String]) -> Result<SongLikeCheckAck> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "check_likes",
        ))
    }

    async fn add_song_to_playlist(
        &self,
        _playlist_id: &str,
        _track_id: &str,
    ) -> Result<PlaylistAddSongAck> {
        Err(error::ProviderError::not_implemented(
            self.id(),
            "add_to_playlist",
        ))
    }
}
