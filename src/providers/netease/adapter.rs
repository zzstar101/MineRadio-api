use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{
    providers::{
        ProviderAdapter, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    services::auth_session,
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlayableState, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, SongLikeAck, SongLikeCheckAck,
        SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability, TrackQualityOption,
    },
};

use super::{
    client::NeteaseClient,
    map::{
        map_hana_lyric_to_payload, map_hana_playlist_to_detail, map_hana_playlist_to_summary,
        map_hana_song_to_track, map_playable, normalize_provider_image_url,
    },
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

const NETEASE_VIP_LEVEL_NAMES: [&str; 11] = [
    "", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖", "拾",
];

#[derive(Clone, Default)]
pub struct NeteaseAdapter {
    client: Arc<NeteaseClient>,
}

impl NeteaseAdapter {
    pub fn new(client: Arc<NeteaseClient>) -> Self {
        Self { client }
    }

    async fn login_status_internal(&self) -> ProviderResult<ProviderLoginStatus> {
        let Some(cookie) = self.client.current_cookie().await else {
            return Ok(ProviderLoginStatus {
                provider: "netease".to_owned(),
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        };
        if cookie.trim().is_empty() {
            return Ok(ProviderLoginStatus {
                provider: "netease".to_owned(),
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        }

        let body = self.client.login_status().await?;
        let profile = body.get("profile");
        let Some(profile) = profile else {
            return Ok(ProviderLoginStatus {
                provider: "netease".to_owned(),
                logged_in: false,
                ..Default::default()
            });
        };

        let user_id = profile
            .get("userId")
            .map(read_id_like)
            .filter(|value| !value.is_empty());
        let vip_info = if let Some(user_id) = user_id.as_deref() {
            Some(self.client.vip_info(user_id).await?)
        } else {
            None
        };

        Ok(map_netease_vip_status(profile, vip_info.as_ref()))
    }
}

#[async_trait]
impl ProviderAdapter for NeteaseAdapter {
    fn id(&self) -> ProviderId {
        "netease".to_owned()
    }

    async fn search(&self, keyword: &str, limit: u32) -> ProviderResult<Vec<Track>> {
        let body = self.client.cloudsearch(keyword, limit).await?;
        let songs = body
            .get("result")
            .and_then(|value| value.get("songs"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        Ok(songs.iter().map(map_hana_song_to_track).collect())
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
                        provider: "netease".to_owned(),
                        logged_in: true,
                        vip_level: Some("none".to_owned()),
                        ..Default::default()
                    })
            } else {
                ProviderLoginStatus {
                    provider: "netease".to_owned(),
                    logged_in: has_cookie,
                    vip_level: Some("none".to_owned()),
                    ..Default::default()
                }
            };
            let vip_level = trial_login_status
                .vip_level
                .clone()
                .unwrap_or_else(|| "none".to_owned());
            let actual_level = netease_actual_level(datum, quality);
            let result = SongUrlResult {
                url: url.map(str::to_owned),
                proxied: false,
                provider: Some("netease".to_owned()),
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
                provider: "netease".to_owned(),
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
                provider: "netease".to_owned(),
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
            provider: "netease".to_owned(),
            track_id: track.source_id.clone(),
            default_quality: qualities.first().map(|item| item.request_quality.clone()),
            qualities,
        })
    }

    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload> {
        let body = match self.client.lyric_new(&track.source_id).await {
            Ok(body) => body,
            Err(_) => self.client.lyric(&track.source_id).await?,
        };
        let lrc = body.get("lrc");
        Ok(map_hana_lyric_to_payload(
            &track.source_id,
            lrc.and_then(|value| value.get("lyric"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
            lrc.and_then(|value| value.get("tlyric"))
                .and_then(|value| value.get("lyric"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
            lrc.and_then(|value| value.get("klyric"))
                .and_then(|value| value.get("lyric"))
                .and_then(Value::as_str),
            lrc.and_then(|value| value.get("yrc"))
                .and_then(|value| value.get("lyric"))
                .and_then(Value::as_str),
        ))
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        self.client.ensure_login().await?;
        let status_body = self.client.login_status().await?;
        let profile = status_body.get("profile");
        let uid = profile
            .and_then(|value| value.get("userId"))
            .map(read_id_like)
            .unwrap_or_default();
        if uid.is_empty() {
            return Ok(Vec::new());
        }
        let body = self.client.user_playlist(&uid, 60).await?;
        Ok(body
            .get("playlist")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| map_hana_playlist_to_summary(item, None))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn playlist_detail(&self, id: &str) -> ProviderResult<PlaylistDetail> {
        let body = self.client.playlist_detail(id).await?;
        let Some(playlist) = body.get("playlist") else {
            return Err(ProviderError {
                code: ProviderErrorCode::NoPlaylist,
                provider: "netease".to_owned(),
                message: format!("netease playlist {id} missing payload"),
                retryable: false,
                action: None,
                raw_message: Some(body.to_string()),
            });
        };
        Ok(map_hana_playlist_to_detail(playlist, Some(id)))
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        self.client.ensure_login().await?;
        Ok(self.client.album_list().await?.standardize())
    }

    async fn album_detail(&self, id: &str) -> ProviderResult<AlbumDetail> {
        Ok(self.client.album_detail(id).await?.standardize())
    }

    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
        self.login_status_internal().await
    }

    async fn logout(&self) -> ProviderResult<()> {
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie("netease").await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> ProviderResult<SongLikeAck> {
        self.client.ensure_login().await?;
        let body = self.client.like(id, liked).await?;
        Ok(SongLikeAck {
            provider: "netease".to_owned(),
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
                provider: "netease".to_owned(),
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
            return Ok(song_like_check_ack("netease", &clean_ids, &liked_ids));
        }

        let status = self.login_status_internal().await?;
        let Some(uid) = status.user_id.filter(|uid| !uid.is_empty()) else {
            return Err(ProviderError {
                code: ProviderErrorCode::LoginRequired,
                provider: "netease".to_owned(),
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
        Ok(song_like_check_ack("netease", &clean_ids, &liked_ids))
    }

    async fn add_song_to_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> ProviderResult<PlaylistAddSongAck> {
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
                provider: "netease".to_owned(),
                message: format!("netease playlist add failed for {track_id}"),
                retryable: false,
                action: None,
                raw_message: Some(final_response.to_string()),
            });
        }
        Ok(PlaylistAddSongAck {
            provider: "netease".to_owned(),
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

fn netease_trial_message(logged_in: bool, vip_level: &str) -> String {
    match (logged_in, vip_level) {
        (true, "svip") => "此歌曲需要单曲、专辑购买或更高权限".to_owned(),
        (true, "vip") => "此歌曲需要 SVIP 或购买 · 当前仅播放试听片段".to_owned(),
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
        provider: "netease".to_owned(),
        message: format!("netease song-url {id} state {state}"),
        retryable: state == PlayableState::LoginRequired,
        action: (state == PlayableState::LoginRequired).then(|| "login".to_owned()),
        raw_message: None,
    }
}

fn map_netease_vip_status(profile: &Value, vip_info_body: Option<&Value>) -> ProviderLoginStatus {
    let candidates = netease_candidate_values(profile, vip_info_body);
    let nickname = profile
        .get("nickname")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let avatar_url = profile
        .get("avatarUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let user_id = profile
        .get("userId")
        .map(read_id_like)
        .filter(|value| !value.is_empty());
    let vip_type = first_number(&candidates, &["vipType", "vip_type", "redVipType"]);
    let vip_level_raw = first_string(
        &candidates,
        &["vipLevel", "vip_level", "levelName", "vipLevelName"],
    );
    let raw_label = usable_vip_label(&first_string(
        &candidates,
        &[
            "vipLabel",
            "vip_label",
            "vipName",
            "memberName",
            "packageName",
            "productName",
            "displayName",
        ],
    ));
    let vip_icon_url = normalize_vip_icon_url(&first_string(
        &candidates,
        &[
            "redVipLevelIcon",
            "vipIconUrl",
            "vipIcon",
            "vipLevelIcon",
            "levelIconUrl",
            "dynamicIconUrl",
            "iconUrl",
            "iconURL",
            "icon",
            "logoUrl",
            "imgUrl",
            "imageUrl",
            "picUrl",
            "levelIcon",
            "rightsIcon",
        ],
    ));
    let text = format!("{} {}", vip_level_raw, raw_label).to_ascii_lowercase();
    let explicit_is_vip = first_flag(&candidates, &["isVip", "vip", "isRedVip", "isMusicPackage"]);
    let explicit_is_svip = first_flag(&candidates, &["isSvip", "svip", "isSuperVip", "isBlackVip"]);
    let vip_level = if text.contains("svip")
        || text.contains("super")
        || text.contains("黑胶svip")
        || text.contains("超级")
        || explicit_is_svip == Some(true)
        || vip_type.unwrap_or_default() >= 10
    {
        "svip"
    } else if text.contains("vip")
        || text.contains("黑胶")
        || text.contains("会员")
        || explicit_is_vip == Some(true)
        || vip_type.unwrap_or_default() > 0
    {
        "vip"
    } else {
        "none"
    };
    let raw_vip_tier = first_number(
        &candidates,
        &[
            "redVipLevel",
            "vipTier",
            "vipLevelValue",
            "vip_level_value",
            "level",
            "grade",
            "growthLevel",
            "musicPackageLevel",
        ],
    )
    .or_else(|| parse_vip_tier_from_text(&vip_level_raw))
    .or_else(|| parse_vip_tier_from_text(&raw_label));
    let vip_tier = (vip_level != "none").then_some(raw_vip_tier).flatten();
    let vip_level_name = vip_level_name_of(vip_tier);
    let base_label = if !raw_label.is_empty() {
        raw_label
    } else if vip_level == "svip" {
        "黑胶SVIP".to_owned()
    } else if vip_level == "vip" {
        "黑胶VIP".to_owned()
    } else {
        String::new()
    };
    let vip_label = append_vip_tier(&base_label, vip_level_name.as_deref());

    ProviderLoginStatus {
        provider: "netease".to_owned(),
        logged_in: true,
        nickname,
        user_id,
        avatar_url,
        vip_type,
        vip_level: Some(vip_level.to_owned()),
        is_vip: Some(matches!(vip_level, "vip" | "svip")),
        is_svip: Some(vip_level == "svip"),
        vip_label: (!vip_label.is_empty()).then_some(vip_label),
        vip_icon: match vip_level {
            "svip" => Some("netease-svip".to_owned()),
            "vip" => Some("netease-vip".to_owned()),
            _ => None,
        },
        vip_icon_url,
        vip_tier,
        vip_level_name,
    }
}

fn netease_candidate_values<'a>(
    profile: &'a Value,
    vip_info_body: Option<&'a Value>,
) -> Vec<&'a Value> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    collect_object_candidates(profile, &mut out, &mut seen, 0);
    if let Some(vip_info_body) = vip_info_body {
        collect_object_candidates(vip_info_body, &mut out, &mut seen, 0);
    }
    out
}

fn collect_object_candidates<'a>(
    value: &'a Value,
    out: &mut Vec<&'a Value>,
    seen: &mut std::collections::HashSet<*const Value>,
    depth: usize,
) {
    if depth > 5 {
        return;
    }
    let ptr = value as *const Value;
    if !seen.insert(ptr) {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_object_candidates(item, out, seen, depth + 1);
            }
        }
        Value::Object(map) => {
            out.push(value);
            for child in map.values() {
                collect_object_candidates(child, out, seen, depth + 1);
            }
        }
        _ => {}
    }
}

fn first_string(candidates: &[&Value], fields: &[&str]) -> String {
    candidates
        .iter()
        .map(|value| read_string_field(value, fields))
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

fn first_number(candidates: &[&Value], fields: &[&str]) -> Option<i64> {
    candidates
        .iter()
        .find_map(|value| read_number_field(value, fields))
}

fn first_flag(candidates: &[&Value], fields: &[&str]) -> Option<bool> {
    candidates
        .iter()
        .find_map(|value| read_flag_field(value, fields))
}

fn read_string_field(value: &Value, fields: &[&str]) -> String {
    for field in fields {
        let Some(value) = value.get(*field) else {
            continue;
        };
        match value {
            Value::String(value) => {
                let text = value.trim();
                if !text.is_empty() {
                    return text.to_owned();
                }
            }
            Value::Number(value) => return value.to_string(),
            _ => {}
        }
    }
    String::new()
}

fn read_number_field(value: &Value, fields: &[&str]) -> Option<i64> {
    for field in fields {
        let Some(value) = value.get(*field) else {
            continue;
        };
        match value {
            Value::Number(number) => {
                if let Some(number) = number.as_i64() {
                    return Some(number);
                }
                if let Some(number) = number.as_u64().and_then(|value| i64::try_from(value).ok()) {
                    return Some(number);
                }
            }
            Value::String(text) => {
                if let Ok(number) = text.trim().parse::<i64>() {
                    return Some(number);
                }
            }
            _ => {}
        }
    }
    None
}

fn read_flag_field(value: &Value, fields: &[&str]) -> Option<bool> {
    for field in fields {
        let Some(value) = value.get(*field) else {
            continue;
        };
        match value {
            Value::Bool(flag) => return Some(*flag),
            Value::Number(number) => {
                if let Some(number) = number.as_i64() {
                    return Some(number > 0);
                }
            }
            Value::String(text) => {
                let text = text.trim().to_ascii_lowercase();
                match text.as_str() {
                    "1" | "true" | "yes" | "y" => return Some(true),
                    "0" | "false" | "no" | "n" | "" => return Some(false),
                    _ => {
                        if let Ok(number) = text.parse::<i64>() {
                            return Some(number > 0);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn vip_level_name_of(tier: Option<i64>) -> Option<String> {
    let tier = tier?;
    if tier <= 0 {
        return None;
    }
    NETEASE_VIP_LEVEL_NAMES
        .get(tier as usize)
        .map(|value| (*value).to_owned())
        .or_else(|| Some(tier.to_string()))
}

fn parse_vip_tier_from_text(text: &str) -> Option<i64> {
    let digits = text
        .split(|ch: char| !ch.is_ascii_digit())
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse::<i64>().ok())
        .filter(|value| *value > 0);
    if digits.is_some() {
        return digits;
    }
    text.chars().find_map(|ch| match ch {
        '一' | '壹' => Some(1),
        '二' | '贰' => Some(2),
        '三' | '叁' => Some(3),
        '四' | '肆' => Some(4),
        '五' | '伍' => Some(5),
        '六' | '陆' => Some(6),
        '七' | '柒' => Some(7),
        '八' | '捌' => Some(8),
        '九' | '玖' => Some(9),
        '十' | '拾' => Some(10),
        _ => None,
    })
}

fn usable_vip_label(label: &str) -> String {
    let cleaned = label.split_whitespace().collect::<String>();
    let lower = cleaned.to_ascii_lowercase();
    if lower.contains("vip")
        || lower.contains("svip")
        || cleaned.contains("黑胶")
        || cleaned.contains("会员")
    {
        cleaned
    } else {
        String::new()
    }
}

fn normalize_vip_icon_url(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with("//")
        || text.len() >= 7 && text[..7].eq_ignore_ascii_case("http://")
        || text.len() >= 8 && text[..8].eq_ignore_ascii_case("https://")
    {
        return Some(normalize_provider_image_url(text));
    }
    if text.starts_with("data:image/") {
        return Some(text.to_owned());
    }
    None
}

fn append_vip_tier(label: &str, tier_name: Option<&str>) -> String {
    let Some(tier_name) = tier_name else {
        return label.to_owned();
    };
    if label.is_empty() || label.contains('·') || label.ends_with(tier_name) {
        return label.to_owned();
    }
    format!("{label}·{tier_name}")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::types::Track;

    use super::{map_netease_vip_status, pick_song_url_datum, response_code};

    #[test]
    fn netease_login_status_merges_vip_detail_label_and_tier() {
        let profile = json!({
            "nickname": "n",
            "avatarUrl": "u",
            "userId": 42,
            "vipType": 11
        });
        let vip_info = json!({
            "vipInfoV2": {
                "data": {
                    "vipLabel": "黑胶SVIP",
                    "redVipLevel": 6
                }
            },
            "vipInfo": {
                "data": {
                    "redVipLevelIcon": "//p1.music.126.net/vip.png"
                }
            }
        });

        let status = map_netease_vip_status(&profile, Some(&vip_info));
        assert_eq!(status.provider, "netease");
        assert!(status.logged_in);
        assert_eq!(status.nickname.as_deref(), Some("n"));
        assert_eq!(status.avatar_url.as_deref(), Some("u"));
        assert_eq!(status.user_id.as_deref(), Some("42"));
        assert_eq!(status.vip_type, Some(11));
        assert_eq!(status.vip_level.as_deref(), Some("svip"));
        assert_eq!(status.is_vip, Some(true));
        assert_eq!(status.is_svip, Some(true));
        assert_eq!(status.vip_label.as_deref(), Some("黑胶SVIP·陆"));
        assert_eq!(
            status.vip_icon_url.as_deref(),
            Some("https://p1.music.126.net/vip.png")
        );
        assert_eq!(status.vip_tier, Some(6));
        assert_eq!(status.vip_level_name.as_deref(), Some("陆"));
    }

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
