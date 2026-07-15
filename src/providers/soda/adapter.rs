use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    providers::{
        ProviderAdapter, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    services::auth_session,
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlaylistDetail, PlaylistSummary, ProviderId,
        ProviderLoginStatus, SongLikeAck, SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track,
        TrackQualityAvailability, TrackQualityOption,
    },
};

use super::{
    client::SodaClient,
    map::{
        map_soda_lyric_to_payload, map_soda_playlist_detail_to_detail, map_soda_playlist_to_summary,
    },
};

#[derive(Clone, Copy)]
struct SodaPlaybackQualityOption {
    level: &'static str,
    soda_level: &'static str,
    aliases: &'static [&'static str],
}

const SODA_PLAYBACK_QUALITY_OPTIONS: [SodaPlaybackQualityOption; 5] = [
    SodaPlaybackQualityOption {
        level: "jymaster",
        soda_level: "spatial",
        aliases: &["spatial"],
    },
    SodaPlaybackQualityOption {
        level: "hires",
        soda_level: "hi_res",
        aliases: &["hi_res", "hi-res", "surround"],
    },
    SodaPlaybackQualityOption {
        level: "lossless",
        soda_level: "highest",
        aliases: &["highest", "lossless"],
    },
    SodaPlaybackQualityOption {
        level: "exhigh",
        soda_level: "higher",
        aliases: &["higher", "exhigh"],
    },
    SodaPlaybackQualityOption {
        level: "standard",
        soda_level: "medium",
        aliases: &["medium", "standard"],
    },
];

#[derive(Clone, Debug)]
struct SodaPlayInfoEntry {
    key: String,
    level: Option<String>,
    quality: String,
    play_url: String,
    play_auth: String,
    filename: Option<String>,
}

#[derive(Clone, Default)]
pub struct SodaAdapter {
    client: Arc<SodaClient>,
}

impl SodaAdapter {
    pub fn new(client: Arc<SodaClient>) -> Self {
        Self { client }
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new(Arc::new(SodaClient::new())))
    }
}

#[async_trait]
impl ProviderAdapter for SodaAdapter {
    fn id(&self) -> ProviderId {
        "soda".to_owned()
    }

    async fn search(&self, keyword: &str, limit: u32) -> ProviderResult<Vec<Track>> {
        let mut t = self.client.search(keyword).await?.standardize();
        t.truncate(limit as usize);
        Ok(t)
    }

    async fn song_url(
        &self,
        track: &Track,
        opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult> {
        self.client.ensure_login().await?;
        let requested = opts
            .and_then(|value| value.quality)
            .unwrap_or_else(|| "exhigh".to_owned());
        let detail = self.client.song_url(&track.source_id).await?;
        let info_url = detail
            .get("track_player")
            .and_then(|player| player.get("url_player_info"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if info_url.is_empty() {
            return Err(unavailable(format!(
                "soda track {} missing url_player_info",
                track.source_id
            )));
        }

        let info_body = self.client.read_json_url(&info_url).await?;
        let play_info_entries = read_soda_play_info_entries(&info_body);
        let play_info = pick_soda_play_info_entry(&play_info_entries, Some(&requested))
            .ok_or_else(|| {
                unavailable(format!("soda track {} missing play info", track.source_id))
            })?;
        let mapped_quality = play_info.level.clone();
        let quality = mapped_quality
            .as_deref()
            .map(|level| soda_quality_label(level, &play_info.quality))
            .unwrap_or_else(|| play_info.quality.clone());

        Ok(SongUrlResult {
            url: Some(format!(
                "/providers/soda/audio-proxy?url={}&playAuth={}",
                urlencoding::encode(&play_info.play_url),
                urlencoding::encode(&play_info.play_auth)
            )),
            proxied: true,
            provider: Some("soda".to_owned()),
            trial: Some(false),
            playable: Some(true),
            level: mapped_quality,
            quality: Some(quality),
            filename: play_info.filename.clone(),
            expires_at: None,
            ..Default::default()
        })
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        let detail = self.client.track_detail(&track.source_id).await?;
        Ok(build_track_quality_availability(&track.source_id, &detail))
    }

    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload> {
        let body = self.client.lyric(&track.source_id).await?;
        let lyric = body.get("lyric");
        let base = lyric
            .and_then(|value| value.get("content"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let trans = lyric
            .and_then(|value| value.get("translations"))
            .and_then(Value::as_object)
            .and_then(|value| value.get("cn"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        Ok(map_soda_lyric_to_payload(&track.source_id, base, trans))
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        let Some(cookie) = self.client.current_cookie().await else {
            return Ok(Vec::new());
        };
        if cookie.trim().is_empty() {
            return Ok(Vec::new());
        }
        let body = self.client.playlist_list().await?;
        Ok(body
            .get("playlists")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|item| map_soda_playlist_to_summary(item, None))
            .collect())
    }

    async fn playlist_detail(&self, id: &str) -> ProviderResult<PlaylistDetail> {
        let body = self.client.playlist_detail(id).await?;
        Ok(map_soda_playlist_detail_to_detail(Some(&body), Some(id)))
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        Ok(self.client.album_list().await?.standardize())
    }

    async fn album_detail(&self, id: &str) -> ProviderResult<AlbumDetail> {
        Ok(self.client.album_detail(id).await?.standardize())
    }

    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
        let Some(cookie) = self.client.current_cookie().await else {
            return Ok(ProviderLoginStatus {
                provider: "soda".to_owned(),
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        };
        if cookie.trim().is_empty() {
            return Ok(ProviderLoginStatus {
                provider: "soda".to_owned(),
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        }
        let body = self.client.login_status().await?;
        let my_info = body.get("my_info");
        let logged_in = body.get("status_code").and_then(Value::as_i64) == Some(0)
            && my_info
                .and_then(|info| info.get("id"))
                .map(value_to_string)
                .filter(|value| !value.is_empty())
                .is_some();
        let mut status = ProviderLoginStatus {
            provider: "soda".to_owned(),
            logged_in,
            ..Default::default()
        };
        if logged_in {
            status.nickname = my_info
                .and_then(|info| info.get("nickname"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            status.user_id = my_info
                .and_then(|info| info.get("id"))
                .map(value_to_string)
                .filter(|value| !value.is_empty());
            status.avatar_url = my_info
                .map(|info| read_soda_avatar_url(info.get("medium_avatar_url")))
                .map(|value| normalize_provider_image_url(&value))
                .filter(|value| !value.is_empty());
            let vip_stage = my_info
                .and_then(|info| info.get("vip_stage"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            let is_vip = my_info
                .and_then(|info| info.get("is_vip"))
                .and_then(Value::as_bool)
                == Some(true);
            let is_svip = is_vip && vip_stage == "svip";
            let vip_level = if is_vip {
                if is_svip { "svip" } else { "vip" }
            } else {
                "none"
            };

            status.vip_type = Some(match vip_level {
                "svip" => 11,
                "vip" => 1,
                _ => 0,
            });
            status.vip_level = Some(vip_level.to_owned());
            status.is_vip = Some(is_vip || is_svip);
            status.is_svip = Some(is_svip);
            status.vip_label = (is_vip && !vip_stage.is_empty()).then_some(vip_stage.clone());
            status.vip_level_name = (!vip_stage.is_empty()).then_some(vip_stage);
        }

        Ok(status)
    }

    async fn logout(&self) -> ProviderResult<()> {
        if self
            .client
            .current_cookie()
            .await
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            return Err(ProviderError::not_implemented(
                "soda".to_owned(),
                "no-session",
            ));
        }
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie("soda").await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> ProviderResult<SongLikeAck> {
        self.client.ensure_login().await?;
        let clean_id = id.trim();
        let (body, status) = self.client.collection_media(clean_id, liked).await?;
        let ok_key = if liked {
            "collected_media"
        } else {
            "deleted_media"
        };
        if body.get(ok_key).is_none() {
            let status_message = body
                .get("status_info")
                .and_then(|value| value.get("status_msg"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let message = status_message
                .map(str::to_owned)
                .unwrap_or_else(|| format!("soda like-song failed with status {status}"));
            return Err(unavailable(message));
        }
        Ok(SongLikeAck {
            provider: "soda".to_owned(),
            id: clean_id.to_owned(),
            liked,
            code: Some(i64::from(status)),
        })
    }

    async fn check_song_likes(&self, ids: &[String]) -> ProviderResult<SongLikeCheckAck> {
        self.client.ensure_login().await?;
        let clean_ids = ids
            .iter()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        let mut liked_ids = Vec::new();
        for id in &clean_ids {
            let body =
                self.client.track_detail(id).await.map_err(|err| {
                    unavailable(format!("soda like-check failed: {}", err.message))
                })?;
            if body
                .get("track")
                .and_then(|track| track.get("state"))
                .and_then(|state| state.get("is_collected"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                liked_ids.push(id.clone());
            }
        }
        Ok(song_like_check_ack("soda", &clean_ids, &liked_ids))
    }
}

fn unavailable(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: "soda".to_owned(),
        message,
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn build_track_quality_availability(track_id: &str, detail: &Value) -> TrackQualityAvailability {
    let soda_track = detail.get("track").filter(|value| value.is_object());
    let label_info = soda_track
        .and_then(|track| track.get("label_info"))
        .filter(|value| value.is_object());
    let mut seen = std::collections::HashSet::new();
    let mut qualities = soda_track
        .and_then(|track| track.get("bit_rates"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|bit_rate| soda_quality_option_from_bit_rate(bit_rate, label_info))
        .filter(|option| seen.insert(option.id.clone()))
        .collect::<Vec<_>>();

    qualities
        .sort_by_key(|option| soda_quality_rank(option.level.as_deref().unwrap_or(&option.id)));

    TrackQualityAvailability {
        provider: "soda".to_owned(),
        track_id: track_id.to_owned(),
        default_quality: qualities
            .iter()
            .find(|quality| quality.request_quality == "exhigh")
            .map(|quality| quality.request_quality.clone())
            .or_else(|| {
                qualities
                    .first()
                    .map(|quality| quality.request_quality.clone())
            }),
        qualities,
    }
}

fn soda_quality_option_from_bit_rate(
    bit_rate: &Value,
    label_info: Option<&Value>,
) -> Option<TrackQualityOption> {
    let raw_quality = read_string(bit_rate.get("quality"));
    if raw_quality.is_empty() || raw_quality.eq_ignore_ascii_case("lossless") {
        return None;
    }
    let level = map_soda_playback_quality(&raw_quality)?;
    let br = read_number_u32(bit_rate.get("br"));
    let size = read_number_u64(bit_rate.get("size"));
    Some(TrackQualityOption {
        provider: "soda".to_owned(),
        id: level.to_owned(),
        label: soda_quality_label(level, &raw_quality),
        detail: Some(soda_quality_detail(&raw_quality, label_info)),
        request_quality: level.to_owned(),
        level: Some(level.to_owned()),
        r#type: Some(raw_quality),
        br: (br > 0).then_some(br),
        size: (size > 0).then_some(size),
        source: "declared".to_owned(),
        ..Default::default()
    })
}

fn soda_quality_detail(raw_quality: &str, label_info: Option<&Value>) -> String {
    let quality = raw_quality.trim().to_lowercase();
    let vip_play_qualities =
        read_soda_quality_list(label_info.and_then(|value| value.get("quality_only_vip_can_play")));
    let vip_download_qualities = read_soda_quality_list(
        label_info.and_then(|value| value.get("quality_only_vip_can_download")),
    );
    let vip_playable = label_info
        .and_then(|value| value.get("only_vip_playable"))
        .and_then(Value::as_bool)
        == Some(true)
        || vip_play_qualities.contains(&quality);
    let vip_download = label_info
        .and_then(|value| value.get("only_vip_download"))
        .and_then(Value::as_bool)
        == Some(true)
        || vip_download_qualities.contains(&quality);
    let mut parts = vec![if vip_playable {
        "仅 VIP 可播放".to_owned()
    } else {
        "可播放".to_owned()
    }];
    if vip_download {
        parts.push("仅VIP可下载".to_owned());
    }
    parts.join(" · ")
}

fn read_soda_quality_list(value: Option<&Value>) -> std::collections::HashSet<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|item| item.trim().to_lowercase())
        .filter(|item| !item.is_empty())
        .collect()
}

fn read_soda_play_info_entries(body: &Value) -> Vec<SodaPlayInfoEntry> {
    let mut seen = std::collections::HashSet::new();
    body.get("Result")
        .and_then(|value| value.get("Data"))
        .and_then(|value| value.get("PlayInfoList"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|play_info| {
            let play_url = read_string(
                play_info
                    .get("MainPlayUrl")
                    .or_else(|| play_info.get("BackupPlayUrl")),
            );
            let play_auth = read_string(play_info.get("PlayAuth"));
            if play_url.is_empty() || play_auth.is_empty() {
                return None;
            }
            let raw_quality = read_string(play_info.get("Quality"));
            let level = map_soda_playback_quality(&raw_quality).map(str::to_owned);
            let key = level
                .clone()
                .unwrap_or_else(|| raw_quality.trim().to_lowercase());
            if key.is_empty() || !seen.insert(key.clone()) {
                return None;
            }
            Some(SodaPlayInfoEntry {
                key,
                level,
                quality: raw_quality,
                play_url,
                play_auth,
                filename: play_info
                    .get("FileID")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            })
        })
        .collect()
}

fn pick_soda_play_info_entry<'a>(
    entries: &'a [SodaPlayInfoEntry],
    requested: Option<&str>,
) -> Option<&'a SodaPlayInfoEntry> {
    if let Some(requested) = requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase)
    {
        if let Some(entry) = entries.iter().find(|entry| entry.key == requested) {
            return Some(entry);
        }
    }
    entries.first()
}

fn map_soda_playback_quality(raw: &str) -> Option<&'static str> {
    let text = raw.trim().to_lowercase();
    for option in SODA_PLAYBACK_QUALITY_OPTIONS {
        let soda_level = option.soda_level.to_lowercase();
        if text == soda_level || text.contains(&soda_level) {
            return Some(option.level);
        }
        if option.aliases.iter().any(|alias| {
            let alias = alias.to_lowercase();
            text == alias || text.contains(&alias)
        }) {
            return Some(option.level);
        }
    }
    if text.contains("master") || text.contains("jymaster") {
        return Some("jymaster");
    }
    if text.contains("320") || text.contains("exhigh") {
        return Some("exhigh");
    }
    if text.contains("hires") || text.contains("hi-res") {
        return Some("hires");
    }
    if text.contains("flac") || text.contains("lossless") || text.contains("sq") {
        return Some("lossless");
    }
    if text.contains("high") {
        return Some("exhigh");
    }
    if text.contains("128") || text.contains("standard") || text.contains("normal") {
        return Some("standard");
    }
    None
}

fn soda_quality_rank(level: &str) -> usize {
    SODA_PLAYBACK_QUALITY_OPTIONS
        .iter()
        .position(|option| option.level == level)
        .unwrap_or(SODA_PLAYBACK_QUALITY_OPTIONS.len())
}

fn soda_quality_label(mapped: &str, raw: &str) -> String {
    match mapped {
        "jymaster" => "录音室音质".to_owned(),
        "hires" => "超清全景声".to_owned(),
        "lossless" => "无损音质".to_owned(),
        "exhigh" => "极高音质".to_owned(),
        "standard" => "标准音质".to_owned(),
        _ => raw.to_owned(),
    }
}

fn read_soda_avatar_url(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    let Some(obj) = value.as_object() else {
        return String::new();
    };
    let url0 = obj
        .get("urls")
        .and_then(Value::as_array)
        .and_then(|urls| urls.first())
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if !url0.is_empty() {
        return url0.to_owned();
    }
    obj.get("uri")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn normalize_provider_image_url(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }
    if let Some(rest) = value.strip_prefix("//") {
        return format!("https://{rest}");
    }
    if value.len() >= 7 && value[..7].eq_ignore_ascii_case("http://") {
        return format!("https://{}", &value[7..]);
    }
    value.to_owned()
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}

fn read_string(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

fn read_number_u32(value: Option<&Value>) -> u32 {
    value
        .and_then(|value| match value {
            Value::Number(value) => value.as_u64(),
            Value::String(value) => value.trim().parse::<u64>().ok(),
            _ => None,
        })
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or_default()
}

fn read_number_u64(value: Option<&Value>) -> u64 {
    value
        .and_then(|value| match value {
            Value::Number(value) => value.as_u64(),
            Value::String(value) => value.trim().parse::<u64>().ok(),
            _ => None,
        })
        .unwrap_or_default()
}

fn song_like_check_ack(provider: &str, ids: &[String], liked_ids: &[String]) -> SongLikeCheckAck {
    let liked_set = liked_ids
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    SongLikeCheckAck {
        provider: provider.to_owned(),
        ids: ids.to_vec(),
        liked: ids
            .iter()
            .map(|id| (id.clone(), liked_set.contains(id)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn soda_play_info_keeps_unmapped_raw_quality() {
        let body = json!({
            "Result": {
                "Data": {
                    "PlayInfoList": [
                        {
                            "Quality": "m4a",
                            "PlayAuth": "play-auth-1",
                            "MainPlayUrl": "https://cdn.example.com/main.m4a",
                            "FileID": "file-1"
                        }
                    ]
                }
            }
        });

        let entries = read_soda_play_info_entries(&body);
        let entry = pick_soda_play_info_entry(&entries, Some("exhigh")).unwrap();

        assert_eq!(entry.level, None);
        assert_eq!(entry.quality, "m4a");
        assert_eq!(entry.filename.as_deref(), Some("file-1"));
    }

    #[test]
    fn soda_play_info_requested_quality_falls_back_to_first_entry() {
        let body = json!({
            "Result": {
                "Data": {
                    "PlayInfoList": [
                        {
                            "Quality": "standard",
                            "PlayAuth": "play-auth-low",
                            "MainPlayUrl": "https://cdn.example.com/low.m4a",
                            "FileID": "low-file"
                        },
                        {
                            "Quality": "exhigh",
                            "PlayAuth": "play-auth-high",
                            "BackupPlayUrl": "https://cdn.example.com/high.m4a",
                            "FileID": "high-file"
                        }
                    ]
                }
            }
        });

        let entries = read_soda_play_info_entries(&body);
        let entry = pick_soda_play_info_entry(&entries, Some("jymaster")).unwrap();

        assert_eq!(entry.level.as_deref(), Some("standard"));
        assert_eq!(entry.quality, "standard");
        assert_eq!(entry.filename.as_deref(), Some("low-file"));
    }

    #[test]
    fn soda_track_qualities_match_ts_shape() {
        let detail = json!({
            "track": {
                "label_info": {
                    "only_vip_download": false,
                    "only_vip_playable": false,
                    "quality_only_vip_can_download": ["spatial"],
                    "quality_only_vip_can_play": ["highest"]
                },
                "bit_rates": [
                    { "br": 132163, "size": 5965060, "quality": "higher" },
                    { "br": 324197, "size": 14631348, "quality": "spatial" },
                    { "br": 0, "size": 0, "quality": "lossless" },
                    { "br": 980000, "size": 44100000, "quality": "highest" }
                ]
            }
        });

        let result = build_track_quality_availability("soda-1", &detail);

        assert_eq!(result.provider, "soda");
        assert_eq!(result.track_id, "soda-1");
        assert_eq!(result.default_quality.as_deref(), Some("exhigh"));
        assert_eq!(
            result
                .qualities
                .iter()
                .map(|quality| quality.request_quality.as_str())
                .collect::<Vec<_>>(),
            vec!["jymaster", "lossless", "exhigh"]
        );
        assert_eq!(
            result
                .qualities
                .iter()
                .map(|quality| quality.r#type.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["spatial", "highest", "higher"]
        );

        let exhigh = result
            .qualities
            .iter()
            .find(|quality| quality.request_quality == "exhigh")
            .unwrap();
        assert_eq!(exhigh.br, Some(132163));
        assert_eq!(exhigh.size, Some(5965060));
        assert!(exhigh.detail.as_deref().unwrap_or_default().contains("VIP") == false);

        let jymaster = result
            .qualities
            .iter()
            .find(|quality| quality.request_quality == "jymaster")
            .unwrap();
        assert!(
            jymaster
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("VIP")
        );

        let lossless = result
            .qualities
            .iter()
            .find(|quality| quality.request_quality == "lossless")
            .unwrap();
        assert!(
            lossless
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("VIP")
        );
    }
}
