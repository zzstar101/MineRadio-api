use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type ProviderId = String;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayableState {
    #[default]
    Unknown,
    Playable,
    LoginRequired,
    VipRequired,
    PaidRequired,
    CopyrightUnavailable,
    TrialOnly,
    Unavailable,
}

impl PlayableState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Playable => "playable",
            Self::LoginRequired => "login_required",
            Self::VipRequired => "vip_required",
            Self::PaidRequired => "paid_required",
            Self::CopyrightUnavailable => "copyright_unavailable",
            Self::TrialOnly => "trial_only",
            Self::Unavailable => "unavailable",
        }
    }
}

impl std::fmt::Display for PlayableState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::PlayableState;

    #[test]
    fn playable_state_uses_frontend_contract_strings() {
        assert_eq!(
            serde_json::to_string(&PlayableState::VipRequired).unwrap(),
            "\"vip_required\""
        );
        assert_eq!(
            serde_json::from_str::<PlayableState>("\"trial_only\"").unwrap(),
            PlayableState::TrialOnly
        );
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VipLevel {
    Svip,
    Vip,
    #[default]
    None,
}

impl VipLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Svip => "svip",
            Self::Vip => "vip",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub provider: ProviderId,
    pub source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_mid: Option<String>,
    pub title: String,
    pub artists: Vec<String>,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub cover_url: String,
    #[serde(default)]
    pub quality_hints: Vec<String>,
    #[serde(default)]
    pub playable_state: PlayableState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artwork_url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongUrlOptions {
    pub quality: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongUrlResult {
    pub url: Option<String>,
    #[serde(default)]
    pub proxied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trial: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub br: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logged_in: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_type: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_level: Option<VipLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_vip: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_svip: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_tier: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_level_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback_key_ready: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restriction: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tried: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qq_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_message: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackQualityOption {
    pub provider: ProviderId,
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub request_quality: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub br: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackQualityAvailability {
    #[serde(default)]
    pub provider: ProviderId,
    #[serde(default)]
    pub track_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_quality: Option<String>,
    #[serde(default)]
    pub qualities: Vec<TrackQualityOption>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricWord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub time_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub c0: usize,
    pub c1: usize,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub char_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<LyricWord>>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricPayload {
    #[serde(default)]
    pub provider: ProviderId,
    #[serde(default)]
    pub track_id: String,
    pub lines: Vec<LyricLine>,
    #[serde(default)]
    pub has_translation: bool,
    #[serde(default)]
    pub is_word_by_word: bool,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistSummary {
    #[serde(default)]
    pub provider: ProviderId,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub cover_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_count: Option<u32>,
    #[serde(default)]
    pub track_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDetail {
    #[serde(default)]
    pub provider: ProviderId,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub cover_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_count: Option<u32>,
    #[serde(default)]
    pub track_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected: Option<bool>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumSummary {
    #[serde(default)]
    pub provider: ProviderId,
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
    #[serde(default)]
    pub cover_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_count: Option<u32>,
    #[serde(default)]
    pub track_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumDetail {
    #[serde(default)]
    pub provider: ProviderId,
    pub id: String,
    pub name: String,
    pub artists: Vec<String>,
    #[serde(default)]
    pub cover_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_count: Option<u32>,
    #[serde(default)]
    pub track_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected: Option<bool>,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderLoginStatus {
    pub provider: ProviderId,
    pub logged_in: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_type: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_level: Option<VipLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_vip: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_svip: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_tier: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vip_level_name: Option<String>,
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
#[serde(rename_all = "camelCase")]
pub struct SongLikeAck {
    #[serde(default)]
    pub provider: ProviderId,
    pub id: String,
    pub liked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongLikeCheckAck {
    #[serde(default)]
    pub provider: ProviderId,
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub liked: HashMap<String, bool>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistAddSongAck {
    #[serde(default)]
    pub provider: ProviderId,
    pub playlist_id: String,
    pub track_id: String,
    #[serde(default)]
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
}
