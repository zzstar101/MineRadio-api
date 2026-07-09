use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    providers::{
        ProviderAdapter, Result,
        error::{ProviderError, ProviderErrorCode},
    },
    services::auth_session,
    types::{
        LyricPayload, PlaylistDetail, PlaylistSummary, ProviderId, ProviderLoginStatus,
        SongLikeAck, SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track,
        TrackQualityAvailability,
    },
};

use super::{
    client::SodaClient,
    map::{
        map_soda_lyric_to_payload, map_soda_playlist_detail_to_detail,
        map_soda_playlist_to_summary, map_soda_song_to_track,
    },
};

const QUALITY_LEVELS: [(&str, &str); 5] = [
    ("spatial", "jymaster"),
    ("hi_res", "hires"),
    ("highest", "lossless"),
    ("higher", "exhigh"),
    ("medium", "standard"),
];

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

    async fn search(&self, keyword: &str, limit: u32) -> Result<Vec<Track>> {
        let body = self.client.search(keyword).await?;
        let tracks = body
            .get("result_groups")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|group| {
                group.get("data")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
            .filter_map(|item| {
                let meta = item.get("meta")?;
                if meta.get("item_type").and_then(Value::as_str) != Some("track") {
                    return None;
                }
                item.get("entity")
                    .and_then(|entity| entity.get("track"))
                    .cloned()
            })
            .take(limit as usize)
            .map(|item| map_soda_song_to_track(&item))
            .collect();
        Ok(tracks)
    }

    async fn song_url(&self, track: &Track, opts: Option<SongUrlOptions>) -> Result<SongUrlResult> {
        ensure_cookie(self.client.current_cookie().await)?;
        let requested = opts
            .and_then(|value| value.quality)
            .unwrap_or_else(|| "exhigh".to_owned());
        let detail = self.client.track_detail(&track.source_id).await?;
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
        let play_info = pick_play_info(&info_body, &requested)
            .or_else(|| first_play_info(&info_body))
            .ok_or_else(|| unavailable(format!("soda track {} missing play info", track.source_id)))?;
        let play_url = play_info
            .get("MainPlayUrl")
            .or_else(|| play_info.get("BackupPlayUrl"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        let play_auth = play_info
            .get("PlayAuth")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if play_url.is_empty() || play_auth.is_empty() {
            return Err(unavailable(format!(
                "soda track {} play info incomplete",
                track.source_id
            )));
        }
        let raw_quality = play_info
            .get("Quality")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_owned();
        let mapped_quality = map_soda_quality(&raw_quality).unwrap_or_else(|| requested.clone());

        Ok(SongUrlResult {
            url: Some(format!(
                "/providers/soda/audio-proxy?url={}&playAuth={}",
                urlencoding::encode(&play_url),
                urlencoding::encode(&play_auth)
            )),
            quality: Some(soda_quality_label(&mapped_quality, &raw_quality)),
            expires_at: None,
        })
    }

    async fn track_qualities(&self, track: &Track) -> Result<TrackQualityAvailability> {
        let detail = self.client.track_detail(&track.source_id).await?;
        let qualities = detail
            .get("track")
            .and_then(|track| track.get("bit_rates"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("quality").and_then(Value::as_str))
            .filter(|value| !value.eq_ignore_ascii_case("lossless"))
            .filter_map(map_soda_quality)
            .collect::<Vec<_>>();

        Ok(TrackQualityAvailability {
            qualities: dedupe(qualities),
        })
    }

    async fn lyric(&self, track: &Track) -> Result<LyricPayload> {
        let body = self.client.track_detail(&track.source_id).await?;
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

    async fn playlist_list(&self) -> Result<Vec<PlaylistSummary>> {
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

    async fn playlist_detail(&self, id: &str) -> Result<PlaylistDetail> {
        let body = self.client.playlist_detail(id).await?;
        Ok(map_soda_playlist_detail_to_detail(Some(&body), Some(id)))
    }

    async fn login_status(&self) -> Result<ProviderLoginStatus> {
        let Some(cookie) = self.client.current_cookie().await else {
            return Ok(ProviderLoginStatus {
                logged_in: false,
                nickname: None,
            });
        };
        if cookie.trim().is_empty() {
            return Ok(ProviderLoginStatus {
                logged_in: false,
                nickname: None,
            });
        }
        let body = self.client.login_status().await?;
        let logged_in = body.get("status_code").and_then(Value::as_i64) == Some(0)
            && body
                .get("my_info")
                .and_then(|info| info.get("id"))
                .map(value_to_string)
                .filter(|value| !value.is_empty())
                .is_some();

        Ok(ProviderLoginStatus {
            logged_in,
            nickname: body
                .get("my_info")
                .and_then(|info| info.get("nickname"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    async fn logout(&self) -> Result<()> {
        ensure_cookie(self.client.current_cookie().await)?;
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie("soda").await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> Result<SongLikeAck> {
        ensure_cookie(self.client.current_cookie().await)?;
        let clean_id = id.trim();
        let (body, status) = self.client.collection_media(clean_id, liked).await?;
        let ok_key = if liked {
            "collected_media"
        } else {
            "deleted_media"
        };
        if body.get(ok_key).is_none() {
            let message = body
                .get("status_info")
                .and_then(|value| value.get("status_msg"))
                .and_then(Value::as_str)
                .unwrap_or("soda like-song failed");
            return Err(unavailable(format!("{message} (status {status})")));
        }
        Ok(SongLikeAck {
            id: clean_id.to_owned(),
            liked,
        })
    }

    async fn check_song_likes(&self, ids: &[String]) -> Result<SongLikeCheckAck> {
        ensure_cookie(self.client.current_cookie().await)?;
        let clean_ids = ids
            .iter()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        let mut liked_ids = Vec::new();
        for id in clean_ids {
            let body = self.client.track_detail(&id).await?;
            if body
                .get("track")
                .and_then(|track| track.get("state"))
                .and_then(|state| state.get("is_collected"))
                .and_then(Value::as_bool)
                == Some(true)
            {
                liked_ids.push(id);
            }
        }
        Ok(SongLikeCheckAck { liked_ids })
    }
}

fn ensure_cookie(cookie: Option<String>) -> Result<()> {
    if cookie.as_deref().map(str::trim).unwrap_or_default().is_empty() {
        return Err(ProviderError {
            code: ProviderErrorCode::LoginRequired,
            provider: "soda".to_owned(),
            message: "soda login required".to_owned(),
            retryable: true,
            action: Some("login".to_owned()),
            raw_message: None,
        });
    }
    Ok(())
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

fn pick_play_info<'a>(body: &'a Value, requested: &str) -> Option<&'a Value> {
    let requested = requested.trim().to_lowercase();
    body.get("Result")
        .and_then(|value| value.get("Data"))
        .and_then(|value| value.get("PlayInfoList"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item.get("Quality")
                    .and_then(Value::as_str)
                    .and_then(map_soda_quality)
                    .map(|quality| quality == requested)
                    .unwrap_or(false)
            })
        })
}

fn first_play_info(body: &Value) -> Option<&Value> {
    body.get("Result")
        .and_then(|value| value.get("Data"))
        .and_then(|value| value.get("PlayInfoList"))
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item.get("MainPlayUrl")
                    .or_else(|| item.get("BackupPlayUrl"))
                    .and_then(Value::as_str)
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
                    && item
                        .get("PlayAuth")
                        .and_then(Value::as_str)
                        .map(|value| !value.trim().is_empty())
                        .unwrap_or(false)
            })
        })
}

fn map_soda_quality(raw: &str) -> Option<String> {
    let text = raw.trim().to_lowercase();
    for (soda, mapped) in QUALITY_LEVELS {
        if text == soda || text.contains(soda) {
            return Some(mapped.to_owned());
        }
    }
    if text.contains("master") {
        return Some("jymaster".to_owned());
    }
    if text.contains("hires") || text.contains("hi-res") {
        return Some("hires".to_owned());
    }
    if text.contains("flac") || text.contains("sq") {
        return Some("lossless".to_owned());
    }
    if text.contains("320") || text.contains("high") {
        return Some("exhigh".to_owned());
    }
    if text.contains("128") || text.contains("normal") {
        return Some("standard".to_owned());
    }
    None
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

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}
