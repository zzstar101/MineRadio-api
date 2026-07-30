use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    parsers::{
        MemchrParsers,
        lrc::LrcParser,
        netease::{NeteaseLrcParser, NeteaseParser},
    },
    providers::{
        ProviderAdapter, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    services::auth_session,
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlayableState, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, SongLikeAck, SongLikeCheckAck,
        SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability, TrackQualityOption,
        VipLevel,
    },
};

use super::{
    client::NeteaseClient,
    map::{map_hana_song_to_track, map_playable},
};

#[derive(Clone, Copy)]
struct QualityCandidate {
    level: &'static str,
    br: u32,
    label: &'static str,
    short: &'static str,
}

const QUALITY_CANDIDATES: [QualityCandidate; 9] = [
    QualityCandidate {
        level: "jymaster",
        br: 1_999_000,
        label: "超清母带",
        short: "母带",
    },
    QualityCandidate {
        level: "dolby",
        br: 1_999_000,
        label: "杜比全景声",
        short: "杜比",
    },
    QualityCandidate {
        level: "sky",
        br: 1_999_000,
        label: "沉浸环绕声",
        short: "沉浸",
    },
    QualityCandidate {
        level: "jyeffect",
        br: 1_999_000,
        label: "高清环绕声",
        short: "环绕",
    },
    QualityCandidate {
        level: "hires",
        br: 1_999_000,
        label: "Hi-Res",
        short: "Hi-Res",
    },
    QualityCandidate {
        level: "lossless",
        br: 1_411_000,
        label: "无损",
        short: "SQ",
    },
    QualityCandidate {
        level: "exhigh",
        br: 999_000,
        label: "极高",
        short: "HQ",
    },
    QualityCandidate {
        level: "higher",
        br: 192_000,
        label: "较高",
        short: "192k",
    },
    QualityCandidate {
        level: "standard",
        br: 128_000,
        label: "标准",
        short: "128k",
    },
];

#[derive(Clone, Default)]
pub struct NeteaseAdapter {
    client: Arc<NeteaseClient>,
    album_cache: Arc<Mutex<HashMap<String, (Vec<Track>, Instant)>>>,
}

impl NeteaseAdapter {
    pub fn new(client: Arc<NeteaseClient>) -> Self {
        Self {
            client,
            album_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn login_status_internal(&self) -> ProviderResult<ProviderLoginStatus> {
        let Some(cookie) = self.client.current_cookie().await else {
            return Ok(ProviderLoginStatus {
                provider: ProviderId::Netease,
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        };
        if cookie.trim().is_empty() {
            return Ok(ProviderLoginStatus {
                provider: ProviderId::Netease,
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        }
        let login_status = self.client.login_status().await?.standardize();
        let Some(user_id) = login_status
            .user_id
            .clone()
            .filter(|user_id| !user_id.is_empty())
        else {
            return Ok(login_status);
        };
        Ok(self
            .client
            .vip_info(&user_id)
            .await?
            .standardize(login_status))
    }
}

#[async_trait]
impl ProviderAdapter for NeteaseAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Netease
    }

    async fn search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<Track>> {
        let body = self.client.cloudsearch(keyword, offset, limit).await?;
        let songs = body
            .get("result")
            .and_then(|value| value.get("songs"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        Ok(songs.iter().map(map_hana_song_to_track).collect())
    }

    async fn search_album(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<AlbumSummary>> {
        Ok(self
            .client
            .search_album(keyword, offset, limit)
            .await?
            .standardize()
            .unwrap_or_default())
    }

    async fn search_playlist(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<PlaylistSummary>> {
        Ok(self
            .client
            .search_playlist(keyword, offset, limit)
            .await?
            .standardize()
            .unwrap_or_default())
    }

    async fn song_url(
        &self,
        track: &Track,
        opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult> {
        let requested = opts
            .and_then(|value| value.quality)
            .unwrap_or_else(|| "hires".to_owned());
        let start_index = QUALITY_CANDIDATES
            .iter()
            .position(|quality| quality.level == requested)
            .unwrap_or(4);
        let has_cookie = self
            .client
            .current_cookie()
            .await
            .is_some_and(|cookie| !cookie.trim().is_empty());
        let mut trial_fallback = None;
        let mut received_datum = false;
        let mut last_state = PlayableState::Unknown;
        let mut last_error = None;

        for quality in QUALITY_CANDIDATES.iter().skip(start_index) {
            let body = match self
                .client
                .song_url_v1(&track.source_id, quality.level)
                .await
            {
                Ok(body) => body,
                Err(_) => match self.client.song_url(&track.source_id, quality.br).await {
                    Ok(body) => body,
                    Err(err) => {
                        last_error = Some(err);
                        continue;
                    }
                },
            };
            let datum = pick_song_url_datum(&body, track);

            let Some(datum) = datum else {
                continue;
            };
            received_datum = true;
            let url = datum.get("url").and_then(Value::as_str);
            let fee = datum.get("fee").and_then(Value::as_i64);
            let code = datum.get("code").and_then(Value::as_i64);
            let free_trial_info = datum.get("freeTrialInfo").filter(|value| !value.is_null());
            let state = map_playable(fee, code, free_trial_info, has_cookie, url);
            last_state = state;
            if state != PlayableState::Playable || url.filter(|value| !value.is_empty()).is_none() {
                continue;
            }
            let trial = free_trial_info.is_some();
            let trial_login_status = if trial {
                self.login_status_internal()
                    .await
                    .unwrap_or(ProviderLoginStatus {
                        provider: ProviderId::Netease,
                        logged_in: true,
                        vip_level: Some(crate::types::VipLevel::None),
                        ..Default::default()
                    })
            } else {
                ProviderLoginStatus {
                    provider: ProviderId::Netease,
                    logged_in: has_cookie,
                    vip_level: Some(crate::types::VipLevel::None),
                    ..Default::default()
                }
            };
            let vip_level = trial_login_status
                .vip_level
                .clone()
                .unwrap_or_else(|| crate::types::VipLevel::None);
            let actual_level = netease_actual_level(datum, quality);
            let result = SongUrlResult {
                url: url.map(str::to_owned),
                proxied: false,
                provider: Some(ProviderId::Netease),
                trial: Some(trial),
                playable: Some(true),
                level: Some(actual_level.clone()),
                quality: Some(netease_quality_label(&actual_level, quality).to_owned()),
                br: datum
                    .get("br")
                    .and_then(Value::as_u64)
                    .and_then(|value| u32::try_from(value).ok()),
                requested_quality: Some(requested.clone()),
                logged_in: Some(trial_login_status.logged_in),
                vip_type: trial_login_status.vip_type,
                vip_level: Some(vip_level.clone()),
                is_vip: trial_login_status.is_vip,
                is_svip: trial_login_status.is_svip,
                vip_label: trial_login_status.vip_label,
                vip_icon: trial_login_status.vip_icon,
                vip_icon_url: trial_login_status.vip_icon_url,
                vip_tier: trial_login_status.vip_tier,
                vip_level_name: trial_login_status.vip_level_name,
                restriction: trial.then(|| netease_trial_restriction(code, fee)),
                reason: trial.then(|| "trial_only".to_owned()),
                message: trial
                    .then(|| netease_trial_message(trial_login_status.logged_in, &vip_level)),
                expires_at: None,
                ..Default::default()
            };
            if trial {
                if trial_fallback.is_none() {
                    trial_fallback = Some(result);
                }
                continue;
            }
            return Ok(result);
        }

        if let Some(result) = trial_fallback {
            return Ok(result);
        }
        if !received_datum {
            if let Some(err) = last_error {
                return Err(err);
            }
            return Err(ProviderError {
                code: ProviderErrorCode::Unavailable,
                provider: ProviderId::Netease,
                message: format!("netease song-url returned no data for {}", track.source_id),
                retryable: false,
                action: None,
                raw_message: None,
            });
        }
        Err(state_error(last_state, &track.source_id))
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        let has_cookie = self
            .client
            .current_cookie()
            .await
            .is_some_and(|cookie| !cookie.trim().is_empty());
        let mut qualities = Vec::new();

        for quality in QUALITY_CANDIDATES {
            let body = match self
                .client
                .song_url_v1(&track.source_id, quality.level)
                .await
            {
                Ok(body) => body,
                Err(_) => match self.client.song_url(&track.source_id, quality.br).await {
                    Ok(body) => body,
                    Err(_) => continue,
                },
            };
            if body.is_null() {
                continue;
            }
            let datum = pick_song_url_datum(&body, track);
            let Some(datum) = datum else {
                continue;
            };
            let url = datum.get("url").and_then(Value::as_str);
            let state = map_playable(
                datum.get("fee").and_then(Value::as_i64),
                datum.get("code").and_then(Value::as_i64),
                datum.get("freeTrialInfo").filter(|value| !value.is_null()),
                has_cookie,
                url,
            );
            if state != PlayableState::Playable || url.filter(|value| !value.is_empty()).is_none() {
                continue;
            }
            let actual_level = netease_actual_level(datum, &quality);
            if qualities
                .iter()
                .any(|option: &TrackQualityOption| option.id == actual_level)
            {
                continue;
            }
            let br = datum
                .get("br")
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(quality.br);
            let media_type = datum
                .get("type")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            qualities.push(TrackQualityOption {
                provider: ProviderId::Netease,
                id: actual_level.clone(),
                label: netease_quality_label(&actual_level, &quality).to_owned(),
                short: Some(netease_quality_short(&actual_level, &quality).to_owned()),
                detail: Some(netease_quality_detail(br, media_type.as_deref())),
                request_quality: actual_level.clone(),
                level: Some(actual_level),
                r#type: media_type,
                br: Some(br),
                source: "resolved".to_owned(),
                ..Default::default()
            });
        }

        qualities.sort_by_key(|option| {
            netease_quality_rank(option.level.as_deref().unwrap_or(&option.id))
        });
        Ok(TrackQualityAvailability {
            provider: ProviderId::Netease,
            track_id: track.source_id.clone(),
            default_quality: qualities.first().map(|item| item.request_quality.clone()),
            qualities,
        })
    }

    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload> {
        let resp = match self.client.lyric_new(&track.source_id).await {
            Ok(resp) => resp,
            Err(_) => self.client.lyric(&track.source_id).await?,
        };

        let lrc_text = resp.lrc.lyric.unwrap_or_default();
        let tlyric_text = resp.tlyric.lyric.unwrap_or_default();
        let yrc_text = resp.yrc.lyric.unwrap_or_default();

        let trans = (!tlyric_text.is_empty())
            .then(|| NeteaseLrcParser {}.parse(tlyric_text).ok())
            .flatten()
            .map(|t| {
                t.into_iter()
                    .map(|line| (line.time_ms, line.text))
                    .collect::<HashMap<_, _>>()
            });

        let (lines, has_translation) = {
            let base_lines = if !yrc_text.is_empty() {
                NeteaseParser
                    .parse(yrc_text)
                    .map_err(|e| invalid_response(e))?
            } else {
                NeteaseLrcParser {}
                    .parse(lrc_text)
                    .map_err(|e| invalid_response(e))?
            };

            match trans {
                Some(trans) => (
                    base_lines
                        .into_iter()
                        .map(|mut line| {
                            line.translation =
                                trans.get(&line.time_ms).cloned().filter(|v| !v.is_empty());
                            line
                        })
                        .collect(),
                    true,
                ),
                None => (base_lines, false),
            }
        };

        let is_word_by_word = lines.iter().any(|line| {
            line.words
                .as_ref()
                .map(|words| !words.is_empty())
                .unwrap_or(false)
        });

        Ok(LyricPayload {
            provider: ProviderId::Netease,
            track_id: track.id.clone(),
            lines,
            has_translation,
            is_word_by_word,
        })
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        self.client.ensure_login().await?;
        let uid = self.client.login_status().await?.standardize().user_id;
        let uid = uid.unwrap_or_default();
        if uid.is_empty() {
            return Err(unavailable("missing uid".to_owned()));
        }
        self.client
            .playlist_list(&uid, 60)
            .await?
            .standardize()
            .ok_or_else(|| no_result("playlist_list"))
    }

    async fn playlist_detail(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<PlaylistDetail> {
        Ok(self
            .client
            .playlist_detail(id, offset, limit)
            .await?
            .standardize())
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        self.client.ensure_login().await?;
        Ok(self
            .client
            .album_list()
            .await?
            .standardize()
            .unwrap_or_default())
    }

    async fn album_detail(&self, id: &str, offset: u32, limit: u32) -> ProviderResult<AlbumDetail> {
        {
            let cache = self.album_cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((tracks, expires_at)) = cache.get(id) {
                if *expires_at > Instant::now() {
                    let start = offset as usize;
                    let end = (start + limit as usize).min(tracks.len());
                    let sliced = if start < tracks.len() {
                        tracks[start..end].to_vec()
                    } else {
                        vec![]
                    };
                    let has_more = (offset + limit) < tracks.len() as u32;
                    return Ok(AlbumDetail {
                        provider: ProviderId::Netease,
                        id: id.to_owned(),
                        name: String::new(),
                        artists: vec![],
                        cover_url: String::new(),
                        track_count: Some(tracks.len() as u32),
                        track_ids: sliced.iter().map(|t| t.source_id.clone()).collect(),
                        collected: None,
                        tracks: sliced,
                        has_more: Some(has_more),
                    });
                }
            }
        }

        let mut detail = self.client.album_detail(id).await?.standardize();

        {
            let mut cache = self.album_cache.lock().unwrap_or_else(|e| e.into_inner());
            cache.insert(
                id.to_owned(),
                (
                    detail.tracks.clone(),
                    Instant::now() + Duration::from_secs(300),
                ),
            );
        }

        let total = detail.tracks.len() as u32;
        let start = offset as usize;
        let end = (start + limit as usize).min(detail.tracks.len());
        if start < detail.tracks.len() {
            detail.tracks = detail.tracks[start..end].to_vec();
            detail.track_ids = detail.track_ids[start..end].to_vec();
        } else {
            detail.tracks = vec![];
            detail.track_ids = vec![];
        }
        detail.has_more = Some((offset + limit) < total);
        Ok(detail)
    }

    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
        self.login_status_internal().await
    }

    async fn logout(&self) -> ProviderResult<()> {
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie(&ProviderId::Netease).await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> ProviderResult<SongLikeAck> {
        self.client.ensure_login().await?;
        let body = self.client.like(id, liked).await?;
        Ok(SongLikeAck {
            provider: ProviderId::Netease,
            id: id.to_owned(),
            liked,
            code: Some(response_code(&body)),
        })
    }

    async fn check_song_likes(&self, ids: &[String]) -> ProviderResult<SongLikeCheckAck> {
        self.client.ensure_login().await?;
        let clean_ids = ids
            .iter()
            .filter(|id| !id.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if clean_ids.is_empty() {
            return Ok(SongLikeCheckAck {
                provider: ProviderId::Netease,
                ids: Vec::new(),
                liked: std::collections::HashMap::new(),
            });
        }

        let liked_ids = match self.client.song_like_check(&clean_ids).await {
            Ok(body) => match body
                .get("data")
                .filter(|value| !value.is_null())
                .or_else(|| body.get("ids").filter(|value| !value.is_null()))
                .unwrap_or(&body)
            {
                Value::Array(items) => items.iter().map(read_id_like).collect::<Vec<_>>(),
                Value::Object(values) => clean_ids
                    .iter()
                    .filter(|id| {
                        values
                            .get(*id)
                            .or_else(|| {
                                let numeric_id = id.parse::<u64>().ok()?.to_string();
                                values.get(&numeric_id)
                            })
                            .is_some_and(json_truthy)
                    })
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            },
            Err(_) => Vec::new(),
        };

        if !liked_ids.is_empty() {
            return Ok(song_like_check_ack(&clean_ids, &liked_ids));
        }

        let status = self.login_status_internal().await?;
        let Some(uid) = status.user_id.filter(|uid| !uid.is_empty()) else {
            return Err(ProviderError {
                code: ProviderErrorCode::LoginRequired,
                provider: ProviderId::Netease,
                message: "netease like-check requires login".to_owned(),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: None,
            });
        };
        let body = self.client.likelist(&uid).await?;
        let liked_ids = body
            .get("ids")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(read_id_like).collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(song_like_check_ack(&clean_ids, &liked_ids))
    }

    async fn update_song_in_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
        adding: bool,
    ) -> ProviderResult<PlaylistAddSongAck> {
        if !adding {
            return Err(ProviderError::not_implemented(
                ProviderId::Netease,
                "del_from_playlist",
            ));
        }
        //未测试
        self.client.ensure_login().await?;
        let primary = self.client.playlist_tracks(playlist_id, track_id).await?;
        let final_response = if is_successful(&primary) {
            primary
        } else {
            self.client
                .playlist_track_add(playlist_id, track_id)
                .await?
        };
        if !is_successful(&final_response) {
            return Err(ProviderError {
                code: ProviderErrorCode::Unavailable,
                provider: ProviderId::Netease,
                message: format!("netease playlist add failed for {track_id}"),
                retryable: false,
                action: None,
                raw_message: Some(final_response.to_string()),
            });
        }
        Ok(PlaylistAddSongAck {
            provider: ProviderId::Netease,
            playlist_id: playlist_id.to_owned(),
            track_id: track_id.to_owned(),
            success: true,
            code: Some(response_code(&final_response)),
        })
    }
}

fn pick_song_url_datum<'a>(body: &'a Value, track: &Track) -> Option<&'a Value> {
    let items = body.get("data")?.as_array()?;
    items
        .iter()
        .find(|item| {
            item.is_object()
                && item.get("id").map(read_id_like).unwrap_or_default() == track.source_id
        })
        .or_else(|| items.first())
        .filter(|item| item.is_object())
}

fn netease_actual_level(datum: &Value, requested: &QualityCandidate) -> String {
    datum
        .get("level")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|level| !level.is_empty())
        .unwrap_or(requested.level)
        .to_owned()
}

fn netease_quality_label<'a>(level: &str, fallback: &'a QualityCandidate) -> &'a str {
    QUALITY_CANDIDATES
        .iter()
        .find(|candidate| candidate.level == level)
        .map(|candidate| candidate.label)
        .unwrap_or(fallback.label)
}

fn netease_quality_short<'a>(level: &str, fallback: &'a QualityCandidate) -> &'a str {
    QUALITY_CANDIDATES
        .iter()
        .find(|candidate| candidate.level == level)
        .map(|candidate| candidate.short)
        .unwrap_or(fallback.short)
}

fn netease_quality_detail(br: u32, media_type: Option<&str>) -> String {
    let kbps = (br.saturating_add(500)) / 1_000;
    match media_type {
        Some(media_type) => format!("{kbps}kbps · {}", media_type.to_ascii_uppercase()),
        None => format!("{kbps}kbps"),
    }
}

fn netease_quality_rank(level: &str) -> usize {
    QUALITY_CANDIDATES
        .iter()
        .position(|candidate| candidate.level == level)
        .unwrap_or(QUALITY_CANDIDATES.len())
}

fn netease_trial_restriction(code: Option<i64>, fee: Option<i64>) -> Value {
    let mut restriction = serde_json::Map::from_iter([
        ("provider".to_owned(), json!("netease")),
        ("category".to_owned(), json!("trial_only")),
        ("action".to_owned(), json!("upgrade")),
        (
            "message".to_owned(),
            json!("网易云仅返回试听片段，完整播放需要会员或购买"),
        ),
    ]);
    if let Some(code) = code {
        restriction.insert("code".to_owned(), json!(code));
    }
    if let Some(fee) = fee {
        restriction.insert("fee".to_owned(), json!(fee));
    }
    Value::Object(restriction)
}

fn netease_trial_message(logged_in: bool, vip_level: &VipLevel) -> String {
    match (logged_in, vip_level) {
        (true, VipLevel::Svip) => "此歌曲需要单曲、专辑购买或更高权限".to_owned(),
        (true, VipLevel::Vip) => "此歌曲需要 SVIP 或购买 · 当前仅播放试听片段".to_owned(),
        (true, _) => "此歌曲需要 VIP · 当前仅播放试听片段".to_owned(),
        (false, _) => "当前未登录 · 仅播放试听片段".to_owned(),
    }
}

fn response_code(body: &Value) -> i64 {
    body.get("code")
        .and_then(Value::as_f64)
        .filter(|code| code.is_finite())
        .map(|code| code.floor() as i64)
        .unwrap_or(200)
}

fn is_successful(body: &Value) -> bool {
    response_code(body) == 200 && !body.get("error").is_some_and(json_truthy)
}

fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn read_id_like(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}

fn song_like_check_ack(ids: &[String], liked_ids: &[String]) -> SongLikeCheckAck {
    let liked_set = liked_ids
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    SongLikeCheckAck {
        provider: ProviderId::Netease,
        ids: ids.to_vec(),
        liked: ids
            .iter()
            .map(|id| (id.clone(), liked_set.contains(id)))
            .collect(),
    }
}

fn invalid_response(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::InvalidResponse,
        provider: ProviderId::Netease,
        message,
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn state_error(state: PlayableState, id: &str) -> ProviderError {
    let code = match state {
        PlayableState::LoginRequired => ProviderErrorCode::LoginRequired,
        PlayableState::VipRequired => ProviderErrorCode::VipRequired,
        PlayableState::PaidRequired => ProviderErrorCode::PaidRequired,
        PlayableState::TrialOnly => ProviderErrorCode::TrialOnly,
        PlayableState::CopyrightUnavailable => ProviderErrorCode::CopyrightUnavailable,
        _ => ProviderErrorCode::Unavailable,
    };
    ProviderError {
        code,
        provider: ProviderId::Netease,
        message: format!("netease song-url {id} state {state}"),
        retryable: state == PlayableState::LoginRequired,
        action: (state == PlayableState::LoginRequired).then(|| "login".to_owned()),
        raw_message: None,
    }
}

fn no_result(action: &str) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::NoResult,
        provider: ProviderId::Netease,
        message: format!("{} no result", action),
        retryable: false,
        action: Some(action.to_string()),
        raw_message: None,
    }
}

fn unavailable(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: ProviderId::Netease,
        message,
        retryable: false,
        action: None,
        raw_message: None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::types::Track;

    use super::{pick_song_url_datum, response_code};

    #[test]
    fn song_url_datum_prefers_the_requested_track_id() {
        let track = Track {
            source_id: "42".to_owned(),
            ..Default::default()
        };
        let body = json!({
            "data": [
                { "id": 7, "url": "https://first" },
                { "id": 42, "url": "https://matched" }
            ]
        });

        let datum = pick_song_url_datum(&body, &track).expect("matching datum");
        assert_eq!(datum["url"], "https://matched");
    }

    #[test]
    fn response_code_defaults_only_when_the_code_field_is_missing_or_non_numeric() {
        assert_eq!(response_code(&json!({ "code": 201 })), 201);
        assert_eq!(response_code(&json!({ "code": "201" })), 200);
    }
}
