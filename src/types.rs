use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type ProviderId = String;

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct Track {
    pub id: String,
    pub provider: ProviderId,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub artwork_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct SongUrlOptions {
    pub quality: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct SongUrlResult {
    pub url: Option<String>,
    pub quality: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct TrackQualityAvailability {
    pub qualities: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct LyricPayload {
    pub lines: Vec<LyricLine>,
    pub raw: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct PlaylistSummary {
    pub id: String,
    pub name: String,
    pub track_count: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct PlaylistDetail {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct ProviderLoginStatus {
    pub logged_in: bool,
    pub nickname: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct ProviderLoginQrKey {
    pub provider: ProviderId,
    pub key: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct ProviderLoginQrImage {
    pub provider: ProviderId,
    pub key: String,
    pub img: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLoginQrCheck {
    pub provider: ProviderId,
    pub key: String,
    pub code: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub logged_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expired: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct SongLikeAck {
    pub id: String,
    pub liked: bool,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct SongLikeCheckAck {
    pub liked_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct PlaylistAddSongAck {
    pub playlist_id: String,
    pub track_id: String,
}
