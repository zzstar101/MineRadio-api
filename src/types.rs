use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayableState {
    #[default]
    Unknown,
    Playable,
    VipRequired,
    PaidRequired,
    CopyrightUnavailable,
    TrialOnly,
}

impl PlayableState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Playable => "playable",
            Self::VipRequired => "vip_required",
            Self::PaidRequired => "paid_required",
            Self::CopyrightUnavailable => "copyright_unavailable",
            Self::TrialOnly => "trial_only",
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
    use super::{PlayableState, SongUrlResult, Track};

    #[test]
    fn playable_state_uses_frontend_contract_strings() {
        assert_eq!(
            serde_json::to_string(&PlayableState::VipRequired).unwrap(),
            "\"vip_required\""
        );
    }

    #[test]
    fn song_url_result_omits_source_track_when_absent() {
        let result = SongUrlResult {
            url: "https://example/song.m4a".to_owned(),
            quality: "standard".to_owned(),
            ..Default::default()
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("sourceTrack").is_none());
    }

    #[test]
    fn song_url_result_serializes_source_track_as_full_camel_case_track() {
        let result = SongUrlResult {
            url: "https://example/song.m4a".to_owned(),
            quality: "standard".to_owned(),
            source_track: Some(Track {
                id: "003aAYWm".to_owned(),
                provider: super::ProviderId::Qq,
                ..Default::default()
            }),
            ..Default::default()
        };
        let json = serde_json::to_value(&result).unwrap();
        let source = json.get("sourceTrack").expect("sourceTrack present");
        assert_eq!(source["provider"], "qq");
        assert_eq!(source["id"], "003aAYWm");
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchType {
    #[default]
    Track,
    Album,
    Artist,
    Playlist,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Qq,
    Netease,
    Soda,
    Kugou,
    Spotify,
    #[default]
    Unknown,
}

impl ProviderId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Qq => "qq",
            Self::Netease => "netease",
            Self::Soda => "soda",
            Self::Kugou => "kugou",
            Self::Spotify => "spotify",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for ProviderId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "qq" => Ok(Self::Qq),
            "netease" => Ok(Self::Netease),
            "soda" => Ok(Self::Soda),
            "kugou" => Ok(Self::Kugou),
            "spotify" => Ok(Self::Spotify),
            "unknown" => Ok(Self::Unknown),
            _ => Err(format!("unknown provider id: {s}")),
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
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct SongUrlOptions {
    pub quality: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongUrlResult {
    pub url: String,
    pub quality: String,
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_range: Option<PreviewRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_track: Option<Track>,
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRange {
    pub start_ms: u64,
    pub end_ms: u64,
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
    /// 是否还有更多数据未拉取
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
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
pub struct SongLikeAck {
    #[serde(default)]
    pub provider: ProviderId,
    pub id: String,
    pub liked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
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

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
/// 决定卡片交互逻辑
pub enum RecommendationCardKind {
    Track,
    Stream,
    Playlist,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
/// 决定ui渲染模块时候的样式
pub enum RecommendationModuleKind {
    Track,
    Mixed,
    Playlist,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationCard {
    pub id: String,
    pub title: String,
    pub subtitle: String, //副标题或介绍
    pub cover_url: String,
    pub collected: Option<bool>,
    pub kind: RecommendationCardKind,
}
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct RecommendationModule {
    pub title: String, //模块名
    pub list: Vec<RecommendationCard>,
    pub kind: RecommendationModuleKind,
}
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
pub struct RecommendationPage {
    pub provider: ProviderId,
    pub list: Vec<RecommendationModule>,
}

impl crate::utils::single_flight::Paginated for PlaylistDetail {
    fn total(&self) -> usize {
        self.tracks.len()
    }

    fn slice_range(&self, start: usize, end: usize) -> Self {
        // tracks 与 track_ids 理论上成对, 钳制越界防反序列化字段缺失时 panic
        fn page_of<T: Clone>(v: &[T], start: usize, end: usize) -> Vec<T> {
            if start >= v.len() {
                Vec::new()
            } else {
                v[start..end.min(v.len())].to_vec()
            }
        }
        let ids = page_of(&self.track_ids, start, end);
        let tracks = page_of(&self.tracks, start, end);
        PlaylistDetail {
            provider: self.provider,
            id: self.id.clone(),
            name: self.name.clone(),
            cover_url: self.cover_url.clone(),
            track_count: self.track_count,
            track_ids: ids,
            collected: self.collected,
            tracks,
            has_more: self.has_more,
        }
    }
}

impl crate::utils::single_flight::Paginated for AlbumDetail {
    fn total(&self) -> usize {
        self.tracks.len()
    }

    fn slice_range(&self, start: usize, end: usize) -> Self {
        // tracks 与 track_ids 理论上成对, 钳制越界防反序列化字段缺失时 panic
        fn page_of<T: Clone>(v: &[T], start: usize, end: usize) -> Vec<T> {
            if start >= v.len() {
                Vec::new()
            } else {
                v[start..end.min(v.len())].to_vec()
            }
        }
        let ids = page_of(&self.track_ids, start, end);
        let tracks = page_of(&self.tracks, start, end);
        AlbumDetail {
            provider: self.provider,
            id: self.id.clone(),
            name: self.name.clone(),
            artists: self.artists.clone(),
            cover_url: self.cover_url.clone(),
            track_count: self.track_count,
            track_ids: ids,
            collected: self.collected,
            tracks,
            has_more: self.has_more,
        }
    }
}
