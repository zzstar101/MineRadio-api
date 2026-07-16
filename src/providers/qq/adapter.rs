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
        AlbumDetail, AlbumSummary, LyricPayload, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, SongUrlOptions, SongUrlResult, Track,
        TrackQualityAvailability, TrackQualityOption,
    },
};

use super::{
    client::QqClient,
    map::{
        map_qq_lyric_to_payload, map_qq_playlist_to_detail, map_qq_playlist_to_detail_official,
        map_qq_playlist_to_summary, map_qq_song_to_track, normalize_provider_image_url,
    },
};

const QQ_QUALITIES: [&str; 5] = ["flac", "ape", "320", "128", "m4a"];
const QQ_PUBLIC_PLAYLIST_TRACK_LIMIT: u32 = 500;

#[derive(Clone, Default)]
pub struct QqAdapter {
    client: Arc<QqClient>,
}

impl QqAdapter {
    pub fn new(client: Arc<QqClient>) -> Self {
        Self { client }
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new(Arc::new(QqClient::new())))
    }
}

#[async_trait]
impl ProviderAdapter for QqAdapter {
    fn id(&self) -> ProviderId {
        "qq".to_owned()
    }

    async fn search(&self, keyword: &str, limit: u32) -> ProviderResult<Vec<Track>> {
        let tracks = self.client.search(keyword, limit).await?.standardize();
        if !tracks.is_empty() {
            return Ok(tracks);
        }

        let list = self.client.smartbox_search(keyword, limit).await?;
        Ok(list.iter().map(map_qq_song_to_track).collect())
    }

    async fn song_url(
        &self,
        track: &Track,
        opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult> {
        let requested = normalize_request_quality(
            opts.and_then(|value| value.quality)
                .unwrap_or_else(|| "hires".to_owned())
                .as_str(),
        );
        let media_mid = track
            .media_mid
            .clone()
            .unwrap_or_else(|| track.source_id.clone());
        let qualities = candidate_qualities(&requested);
        let cookie = self.client.current_cookie().await.unwrap_or_default();
        let has_cookie = !cookie.trim().is_empty();
        let has_playback_key = QqClient::has_playback_key(&cookie);
        let mut last_error = None;

        for quality in qualities {
            let filename = QqClient::filename_for_quality(&media_mid, quality);
            match self
                .client
                .song_url(&track.source_id, quality, &filename)
                .await
            {
                Ok(body) => {
                    if let Some(url) = qq_song_url_info(&body) {
                        return Ok(SongUrlResult {
                            url: Some(url),
                            proxied: false,
                            provider: Some("qq".to_owned()),
                            trial: Some(false),
                            playable: Some(true),
                            level: Some(quality.to_owned()),
                            quality: Some(qq_quality_label(quality).to_owned()),
                            filename: Some(filename),
                            requested_quality: Some(requested.clone()),
                            expires_at: None,
                            ..Default::default()
                        });
                    }
                    if let Some(error) = qq_song_url_restriction(
                        &body,
                        &track.source_id,
                        has_cookie,
                        has_playback_key,
                    ) {
                        return Err(error);
                    }
                    last_error = Some(format!("no url for quality {quality}"));
                }
                Err(err) => last_error = Some(err.message),
            }
        }

        if !has_cookie {
            return Err(ProviderError {
                code: ProviderErrorCode::LoginRequired,
                provider: "qq".to_owned(),
                message: format!("qq song-url {} requires cookie", track.source_id),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: None,
            });
        }

        Err(ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: "qq".to_owned(),
            message: last_error
                .unwrap_or_else(|| format!("qq song-url {} returned no url", track.source_id)),
            retryable: false,
            action: None,
            raw_message: None,
        })
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        let body = self.client.song_detail(&track.source_id).await?;
        let file = find_file_object(&body);
        let qualities: Vec<TrackQualityOption> = QQ_QUALITIES
            .into_iter()
            .filter(|quality| file_supports_quality(file, quality))
            .map(|quality| TrackQualityOption {
                provider: "qq".to_owned(),
                id: quality.to_owned(),
                label: qq_quality_label(quality).to_owned(),
                request_quality: quality.to_owned(),
                level: Some(quality.to_owned()),
                source: "declared".to_owned(),
                ..Default::default()
            })
            .collect();
        Ok(TrackQualityAvailability {
            provider: "qq".to_owned(),
            track_id: track.source_id.clone(),
            default_quality: qualities.first().map(|item| item.request_quality.clone()),
            qualities,
        })
    }

    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload> {
        let mut body = self.client.lyric(&track.source_id).await?;
        let mut source = "qq-musicu";
        if body
            .get("lyric")
            .and_then(Value::as_str)
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
        {
            if let Ok(legacy) = self.client.legacy_lyric(&track.source_id).await {
                if legacy
                    .get("lyric")
                    .and_then(Value::as_str)
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
                {
                    body = legacy;
                    source = "qq-legacy";
                }
            }
        }

        Ok(map_qq_lyric_to_payload(
            &track.source_id,
            body.get("lyric")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            body.get("trans")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            body.get("qrc").and_then(Value::as_str).unwrap_or_default(),
            Some(source),
        ))
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        let cookie = self.client.current_cookie().await;
        let Some(_) = cookie.filter(|cookie| !cookie.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        let euin = self.client.euin().await;
        let Some(euin) = euin else {
            return Ok(Vec::new());
        };
        let created = self.client.user_songlists(&euin).await.ok();
        let collected = self.client.user_collect_songlists(&euin).await.ok();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();

        if let Some(created) = created {
            if let Some(mm) = created.get("music.musicasset.PlaylistBaseRead.GetPlaylistByUin") {
                if let Some(data) = mm.get("data") {
                    if let Some(list) = data.get("v_playlist").and_then(Value::as_array) {
                        for l in list {
                            if let Some(id) = l.get("tid").and_then(Value::as_u64) {
                                if let Some(name) = l.get("dirName").and_then(Value::as_str) {
                                    out.push(PlaylistSummary {
                                        provider: "qq".to_owned(),
                                        id: id.to_string(),
                                        name: name.to_string(),
                                        cover_url: l
                                            .get("picUrl")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_string(),
                                        track_count: Some(
                                            l.get("songNum").and_then(Value::as_u64).unwrap_or(0)
                                                as u32,
                                        ),
                                        track_ids: vec![],
                                        collected: Some(true),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(collected) = collected {
            for item in read_playlist_list(&collected) {
                let summary = map_qq_playlist_to_summary(item, None);
                if !summary.id.is_empty()
                    && !is_qzone_background_playlist(&summary, item)
                    && seen.insert(summary.id.clone())
                {
                    out.push(summary);
                }
            }
        }

        out.sort_by_key(|summary| !is_favorite_playlist(summary));
        Ok(out)
    }

    async fn playlist_detail(&self, id: &str) -> ProviderResult<PlaylistDetail> {
        let official = self
            .client
            .official_playlist_detail(id, QQ_PUBLIC_PLAYLIST_TRACK_LIMIT)
            .await?;
        let fallback = official
            .get("req_0")
            .and_then(|value| value.get("data"))
            .filter(|value| {
                value
                    .get("songlist")
                    .and_then(Value::as_array)
                    .map(|items| !items.is_empty())
                    .unwrap_or(false)
            });
        if let Some(fallback) = fallback {
            let q = map_qq_playlist_to_detail_official(Some(fallback), Some(id));
            return Ok(q);
        }
        //后续将移除老接口调用
        let body = self.client.playlist_detail(id).await?;
        let first = body
            .get("cdlist")
            .and_then(Value::as_array)
            .and_then(|items| items.first());

        let Some(first) = first else {
            return Err(ProviderError {
                code: ProviderErrorCode::NoPlaylist,
                provider: "qq".to_owned(),
                message: format!("qq playlist {id} missing payload"),
                retryable: false,
                action: None,
                raw_message: Some(body.to_string()),
            });
        };

        Ok(map_qq_playlist_to_detail(Some(first), Some(id)))
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        Ok(self.client.album_list().await?.standardize())
    }

    async fn album_detail(&self, id: &str) -> ProviderResult<AlbumDetail> {
        Ok(self.client.album_detail(id).await?.standardize())
    }

    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
        let cookie = self.client.current_cookie().await;
        let Some(cookie) = cookie.filter(|cookie| !cookie.trim().is_empty()) else {
            return Ok(ProviderLoginStatus {
                provider: "qq".to_owned(),
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        };
        let euin = self.client.euin().await;
        let Some(euin) = euin else {
            return Ok(ProviderLoginStatus {
                provider: "qq".to_owned(),
                logged_in: true,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        };

        let vip_info = self.client.vip_info_with_cookie(&euin, &cookie).await.ok();
        match self.client.login_status_with_cookie(&euin, &cookie).await {
            Ok(body) => Ok(map_qq_login_status(
                Some(&body),
                vip_info.as_ref(),
                Some(&euin),
            )),
            Err(_) => {
                if let Some(vip_info) = vip_info.as_ref() {
                    Ok(map_qq_login_status(None, Some(vip_info), Some(&euin)))
                } else {
                    Ok(ProviderLoginStatus {
                        provider: "qq".to_owned(),
                        logged_in: true,
                        user_id: Some(euin),
                        ..Default::default()
                    })
                }
            }
        }
    }

    async fn logout(&self) -> ProviderResult<()> {
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie("qq").await;
        Ok(())
    }

    async fn add_song_to_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> ProviderResult<PlaylistAddSongAck> {
        self.client.ensure_login().await?;
        let body = self
            .client
            .add_song_to_playlist(playlist_id, track_id)
            .await?;
        let code = body
            .get("result")
            .or_else(|| body.get("code"))
            .and_then(Value::as_i64)
            .unwrap_or_default();
        if matches!(code, 0 | 100) {
            return Ok(PlaylistAddSongAck {
                provider: "qq".to_owned(),
                playlist_id: playlist_id.to_owned(),
                track_id: track_id.to_owned(),
                success: true,
                code: Some(code),
            });
        }
        if matches!(code, 301 | 1000) {
            return Err(ProviderError {
                code: ProviderErrorCode::LoginRequired,
                provider: "qq".to_owned(),
                message: format!("qq playlist {playlist_id} add-song requires cookie"),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: Some(body.to_string()),
            });
        }
        Err(ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: "qq".to_owned(),
            message: body
                .get("errMsg")
                .or_else(|| body.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("qq add-song failed")
                .to_owned(),
            retryable: false,
            action: None,
            raw_message: Some(body.to_string()),
        })
    }
}

fn normalize_request_quality(requested: &str) -> String {
    match requested.trim().to_lowercase().as_str() {
        "jymaster" | "hires" | "lossless" | "sq" => "flac".to_owned(),
        "exhigh" | "high" | "hq" => "320".to_owned(),
        "standard" | "normal" | "std" => "128".to_owned(),
        "aac" => "m4a".to_owned(),
        other => other.to_owned(),
    }
}

fn read_playlist_list(body: &Value) -> Vec<&Value> {
    body.get("list")
        .and_then(Value::as_array)
        .or_else(|| {
            body.get("data")
                .and_then(|value| value.get("list"))
                .and_then(Value::as_array)
        })
        .or_else(|| {
            body.get("data")
                .and_then(|value| value.get("disslist"))
                .and_then(Value::as_array)
        })
        .or_else(|| {
            body.get("data")
                .and_then(|value| value.get("cdlist"))
                .and_then(Value::as_array)
        })
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn is_favorite_playlist(summary: &PlaylistSummary) -> bool {
    let name = summary.name.trim();
    name.contains("我喜欢") || name.contains("我的喜欢") || name.eq_ignore_ascii_case("liked songs")
}

fn is_qzone_background_playlist(summary: &PlaylistSummary, raw: &Value) -> bool {
    let creator = raw
        .get("hostname")
        .or_else(|| raw.get("nick"))
        .or_else(|| raw.get("creator"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let text = format!("{} {}", summary.name, creator).to_lowercase();
    text.contains("qzone") || text.contains("空间") || text.contains("背景音乐")
}

fn candidate_qualities(requested: &str) -> Vec<&'static str> {
    let start = QQ_QUALITIES
        .iter()
        .position(|quality| *quality == requested)
        .unwrap_or(0);
    QQ_QUALITIES[start..].to_vec()
}

fn qq_quality_label(quality: &str) -> &'static str {
    match quality {
        "flac" => "FLAC",
        "ape" => "APE",
        "320" => "320k MP3",
        "128" => "128k MP3",
        "m4a" => "AAC",
        _ => "QQ",
    }
}

fn qq_song_url_info(body: &Value) -> Option<String> {
    let data = body
        .get("req_0")
        .and_then(|value| value.get("data"))
        .or_else(|| body.get("data"))?;
    let info = data
        .get("midurlinfo")
        .and_then(Value::as_array)
        .and_then(|items| {
            items.iter().find(|item| {
                item.get("purl")
                    .and_then(Value::as_str)
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false)
            })
        })?;
    let purl = info.get("purl").and_then(Value::as_str)?.trim();
    if purl.is_empty() {
        return None;
    }
    if purl.starts_with("http://") || purl.starts_with("https://") {
        return Some(purl.to_owned());
    }
    let sip = data
        .get("sip")
        .and_then(Value::as_array)
        .and_then(|items| items.iter().find_map(Value::as_str))
        .unwrap_or("https://ws.stream.qqmusic.qq.com/");
    Some(format!("{sip}{purl}"))
}

fn qq_song_url_restriction(
    body: &Value,
    track_id: &str,
    has_cookie: bool,
    has_playback_key: bool,
) -> Option<ProviderError> {
    let info = body
        .get("req_0")
        .and_then(|value| value.get("data"))
        .and_then(|value| value.get("midurlinfo"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .or_else(|| body.as_object().map(|_| body))?;
    let code = info
        .get("result")
        .or_else(|| info.get("code"))
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let raw_message = info
        .get("msg")
        .or_else(|| info.get("tips"))
        .or_else(|| info.get("errmsg"))
        .or_else(|| info.get("message"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);

    if !has_cookie {
        return Some(ProviderError {
            code: ProviderErrorCode::LoginRequired,
            provider: "qq".to_owned(),
            message: format!("qq song-url {track_id} requires cookie"),
            retryable: true,
            action: Some("login".to_owned()),
            raw_message,
        });
    }

    if code == 104003 && !has_playback_key {
        return Some(ProviderError {
            code: ProviderErrorCode::LoginRequired,
            provider: "qq".to_owned(),
            message: "qq playback authorization required".to_owned(),
            retryable: true,
            action: Some("login".to_owned()),
            raw_message,
        });
    }

    let lower = raw_message.as_deref().unwrap_or_default().to_lowercase();
    if lower.contains("vip")
        || lower.contains("pay")
        || lower.contains("付费")
        || lower.contains("会员")
    {
        return Some(ProviderError {
            code: ProviderErrorCode::PaidRequired,
            provider: "qq".to_owned(),
            message: raw_message
                .clone()
                .unwrap_or_else(|| "qq paid playback required".to_owned()),
            retryable: false,
            action: Some("upgrade".to_owned()),
            raw_message,
        });
    }

    if code == 104003 {
        return Some(ProviderError {
            code: ProviderErrorCode::CopyrightUnavailable,
            provider: "qq".to_owned(),
            message: raw_message
                .clone()
                .unwrap_or_else(|| format!("qq song-url {track_id} unavailable")),
            retryable: false,
            action: Some("switch_source".to_owned()),
            raw_message,
        });
    }

    None
}

fn find_file_object(body: &Value) -> Option<&Value> {
    if let Some(file) = body
        .get("songinfo")
        .and_then(|value| value.get("data"))
        .and_then(|value| value.get("track_info"))
        .and_then(|value| value.get("file"))
    {
        return Some(file);
    }
    body.get("songinfo")
        .and_then(|value| value.get("data"))
        .and_then(|value| value.get("file"))
        .or_else(|| body.get("file"))
}

fn file_supports_quality(file: Option<&Value>, quality: &str) -> bool {
    let Some(file) = file else {
        return false;
    };
    let fields = match quality {
        "flac" => &["size_flac"][..],
        "ape" => &["size_ape"][..],
        "320" => &["size_320mp3"][..],
        "128" => &["size_128mp3"][..],
        "m4a" => &["size_96aac", "size_192aac", "size_48aac"][..],
        _ => &[][..],
    };
    fields.iter().any(|field| {
        file.get(*field)
            .and_then(Value::as_u64)
            .map(|value| value > 0)
            .unwrap_or(false)
    })
}

#[cfg(test)]
fn qq_login_nickname(
    body: Option<&Value>,
    vip_info: Option<&Value>,
    user_id: &str,
) -> Option<String> {
    body.and_then(|value| {
        value
            .get("data")
            .and_then(|value| value.get("creator"))
            .and_then(|value| value.get("nick"))
            .and_then(Value::as_str)
            .or_else(|| {
                value
                    .get("data")
                    .and_then(|value| value.get("creator"))
                    .and_then(|value| value.get("hostname"))
                    .and_then(Value::as_str)
            })
    })
    .or_else(|| {
        vip_info
            .and_then(|value| value.get("getNickHead"))
            .and_then(|value| value.get("data"))
            .and_then(|value| value.get("map_userinfo"))
            .and_then(|value| value.get(user_id))
            .and_then(|value| value.get("nick"))
            .and_then(Value::as_str)
    })
    .map(str::to_owned)
}

#[cfg(test)]
fn qq_login_avatar_url(
    body: Option<&Value>,
    vip_info: Option<&Value>,
    user_id: &str,
) -> Option<String> {
    body.and_then(|value| {
        value
            .get("data")
            .and_then(|value| value.get("creator"))
            .and_then(|value| {
                value
                    .get("headpic")
                    .or_else(|| value.get("pic"))
                    .or_else(|| value.get("avatarUrl"))
            })
            .and_then(Value::as_str)
    })
    .or_else(|| {
        vip_info
            .and_then(|value| value.get("getNickHead"))
            .and_then(|value| value.get("data"))
            .and_then(|value| value.get("map_userinfo"))
            .and_then(|value| value.get(user_id))
            .and_then(|value| {
                value
                    .get("headurl")
                    .or_else(|| value.get("picurl"))
                    .or_else(|| value.get("avatarUrl"))
            })
            .and_then(Value::as_str)
    })
    .map(str::to_owned)
    .filter(|value| !value.is_empty())
}

const QQ_VIP_LEVEL_NAMES: [&str; 11] = [
    "", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖", "拾",
];

fn qq_login_profile_candidates<'a>(
    body: Option<&'a Value>,
    vip_info: Option<&'a Value>,
    fallback_user_id: Option<&str>,
) -> Vec<&'a Value> {
    let mut candidates = Vec::new();

    if let Some(vip_info) = vip_info {
        if let Some(icon_list) = vip_info
            .get("getVipIcon")
            .and_then(|value| value.get("data"))
            .and_then(|value| value.get("UserInfoUI"))
            .and_then(|value| value.get("iconlist"))
            .and_then(Value::as_array)
        {
            for item in icon_list {
                push_profile_candidate(&mut candidates, Some(item));
            }
        }
        push_mapped_profile_candidates(
            &mut candidates,
            vip_info
                .get("getVipInfo")
                .and_then(|value| value.get("data"))
                .and_then(|value| value.get("infoMap")),
            fallback_user_id,
        );
        push_mapped_profile_candidates(
            &mut candidates,
            vip_info
                .get("getNickHead")
                .and_then(|value| value.get("data"))
                .and_then(|value| value.get("map_userinfo")),
            fallback_user_id,
        );
        push_profile_candidate(
            &mut candidates,
            vip_info.get("data").and_then(|value| value.get("creator")),
        );
        push_profile_candidate(&mut candidates, vip_info.get("creator"));
        push_profile_candidate(
            &mut candidates,
            vip_info.get("data").and_then(|value| value.get("user")),
        );
        push_profile_candidate(
            &mut candidates,
            vip_info.get("data").and_then(|value| value.get("profile")),
        );
        push_profile_candidate(&mut candidates, vip_info.get("user"));
        push_profile_candidate(&mut candidates, vip_info.get("profile"));
        push_profile_candidate(&mut candidates, vip_info.get("data"));
        push_profile_candidate(&mut candidates, Some(vip_info));
    }

    if let Some(body) = body {
        push_profile_candidate(
            &mut candidates,
            body.get("data").and_then(|value| value.get("creator")),
        );
        push_profile_candidate(&mut candidates, body.get("creator"));
        push_profile_candidate(
            &mut candidates,
            body.get("data").and_then(|value| value.get("user")),
        );
        push_profile_candidate(
            &mut candidates,
            body.get("data").and_then(|value| value.get("profile")),
        );
        push_profile_candidate(&mut candidates, body.get("user"));
        push_profile_candidate(&mut candidates, body.get("profile"));
        push_profile_candidate(&mut candidates, body.get("data"));
        push_profile_candidate(&mut candidates, Some(body));
    }

    candidates
}

fn push_mapped_profile_candidates<'a>(
    candidates: &mut Vec<&'a Value>,
    map: Option<&'a Value>,
    fallback_user_id: Option<&str>,
) {
    let Some(map) = map.and_then(Value::as_object) else {
        return;
    };
    if let Some(user_id) = fallback_user_id {
        if let Some(value) = map.get(user_id) {
            push_profile_candidate(candidates, Some(value));
            return;
        }
    }
    for value in map.values() {
        push_profile_candidate(candidates, Some(value));
    }
}

fn push_profile_candidate<'a>(candidates: &mut Vec<&'a Value>, value: Option<&'a Value>) {
    let Some(value) = value else {
        return;
    };
    if !value.is_object() {
        return;
    }
    if candidates
        .iter()
        .any(|current| std::ptr::eq(*current, value))
    {
        return;
    }
    candidates.push(value);
}

fn read_string_field(value: &Value, fields: &[&str]) -> String {
    for field in fields {
        let Some(value) = value.get(*field) else {
            continue;
        };
        let text = match value {
            Value::String(value) => value.trim().to_owned(),
            Value::Number(value) => value.to_string(),
            _ => String::new(),
        };
        if !text.is_empty() {
            return text;
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

fn vip_level_name_of(tier: Option<i64>) -> Option<String> {
    let tier = tier?;
    if tier <= 0 {
        return None;
    }
    QQ_VIP_LEVEL_NAMES
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

fn append_vip_tier(label: &str, tier_name: Option<&str>) -> String {
    let Some(tier_name) = tier_name else {
        return label.to_owned();
    };
    if label.is_empty() || label.contains('·') || label.ends_with(tier_name) {
        return label.to_owned();
    }
    format!("{label}·{tier_name}")
}

fn normalize_vip_icon_url(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with("//") || text.starts_with("http://") || text.starts_with("https://") {
        return Some(normalize_provider_image_url(text));
    }
    if text.starts_with("data:image/") {
        return Some(text.to_owned());
    }
    None
}

fn qq_vip_badge_icon_from_url(value: &str) -> Option<(String, String, Option<i64>)> {
    let url = normalize_vip_icon_url(value)?;
    let lower = url.to_ascii_lowercase();
    let marker = lower.rfind('/')?;
    let tail = &lower[marker + 1..];
    let level = if tail.starts_with("svip") {
        "svip"
    } else if tail.starts_with("vip") {
        "vip"
    } else {
        return None;
    };
    let digits = tail[level.len()..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    let tier = digits.parse::<i64>().ok();
    Some((url, level.to_owned(), tier))
}

fn first_qq_vip_badge_icon(candidates: &[&Value]) -> Option<(String, String, Option<i64>)> {
    for value in candidates {
        let badge = qq_vip_badge_icon_from_url(&read_string_field(
            value,
            &[
                "srcUrl",
                "src",
                "vipIconUrl",
                "vipIcon",
                "iconUrl",
                "iconurl",
                "iconURL",
                "icon",
                "logoUrl",
                "imgUrl",
                "imageUrl",
                "picUrl",
                "levelIcon",
            ],
        ));
        if badge.is_some() {
            return badge;
        }
    }
    None
}

fn qq_official_vip_icon_url(level: &str, tier: Option<i64>) -> Option<String> {
    if level == "none" {
        return None;
    }
    let badge_tier = tier.unwrap_or(1).clamp(1, 9);
    Some(format!(
        "https://y.qq.com/mediastyle/lv-icon/v14/2x/{level}{badge_tier}.png"
    ))
}

fn map_qq_login_status(
    body: Option<&Value>,
    vip_info: Option<&Value>,
    fallback_user_id: Option<&str>,
) -> ProviderLoginStatus {
    let candidates = qq_login_profile_candidates(body, vip_info, fallback_user_id);
    let mut status = ProviderLoginStatus {
        provider: "qq".to_owned(),
        logged_in: true,
        ..Default::default()
    };

    let nickname = first_string(&candidates, &["nick", "nickname", "name", "hostname"]);
    if !nickname.is_empty() {
        status.nickname = Some(nickname);
    }
    let avatar = first_string(
        &candidates,
        &[
            "headpic",
            "headurl",
            "avatarUrl",
            "avatar",
            "logo",
            "pic",
            "picurl",
            "head_pic",
            "avatar_url",
        ],
    );
    if let Some(avatar_url) = normalize_vip_icon_url(&avatar) {
        status.avatar_url = Some(avatar_url);
    }
    let user_id = {
        let mapped = first_string(
            &candidates,
            &["userid", "hostuin", "uin", "qq", "id", "musicid"],
        );
        if mapped.is_empty() {
            fallback_user_id.unwrap_or_default().to_owned()
        } else {
            mapped
        }
    };
    if !user_id.is_empty() {
        status.user_id = Some(user_id);
    }

    apply_qq_vip_status(&mut status, &candidates);
    status
}

fn apply_qq_vip_status(status: &mut ProviderLoginStatus, candidates: &[&Value]) {
    let badge_icon = first_qq_vip_badge_icon(candidates);
    let explicit_level = first_string(
        candidates,
        &[
            "vipLevel",
            "level",
            "vip_level",
            "vipName",
            "vip_label",
            "vipLabel",
        ],
    );
    let explicit_type = first_number(candidates, &["vipType", "vip_type", "iVipType", "type"]);
    let super_vip = first_flag(
        candidates,
        &[
            "iSuperVip",
            "iNewSuperVip",
            "HugeVip",
            "hugeVip",
            "iHugeVip",
            "svip",
            "superVip",
            "isSvip",
            "isSuperVip",
            "itwelve",
            "twelve",
        ],
    );
    let normal_vip = first_flag(
        candidates,
        &[
            "iVipFlag",
            "iNewVip",
            "iNewVipFlag",
            "iMusicVip",
            "iVip",
            "vipFlag",
            "vip",
            "isVip",
            "ieight",
            "eight",
        ],
    );
    let super_tier = first_number(
        candidates,
        &[
            "iSuperVipLevel",
            "iSvipLevel",
            "iNewSuperVipLevel",
            "iNewSvipLevel",
            "superVipLevel",
            "svipLevel",
            "itwelveLevel",
            "twelveLevel",
            "iCurLevel",
            "iMusicLevel",
        ],
    );
    let normal_tier = first_number(
        candidates,
        &[
            "iVipLevel",
            "iNewVipLevel",
            "vipLevelValue",
            "vip_level_value",
            "greenVipLevel",
            "iGreenVipLevel",
            "musicVipLevel",
            "ieightLevel",
            "eightLevel",
            "iMusicLevel",
            "iCurLevel",
            "iLevel",
            "level",
        ],
    );
    let vip_icon_url = normalize_vip_icon_url(&first_string(
        candidates,
        &[
            "vipIconUrl",
            "vipIcon",
            "iconUrl",
            "iconurl",
            "iconURL",
            "icon",
            "logoUrl",
            "imgUrl",
            "imageUrl",
            "picUrl",
            "levelIcon",
        ],
    ));

    let saw_vip_signal = !explicit_level.is_empty()
        || explicit_type.is_some()
        || super_vip.is_some()
        || normal_vip.is_some()
        || super_tier.is_some()
        || normal_tier.is_some()
        || vip_icon_url.is_some()
        || badge_icon.is_some();
    if !saw_vip_signal {
        return;
    }

    let lower_level = explicit_level.to_ascii_lowercase();
    let level = if let Some((_, badge_level, _)) = badge_icon.as_ref() {
        badge_level.clone()
    } else if lower_level.contains("svip")
        || lower_level.contains("super")
        || lower_level.contains("超级会员")
        || super_vip == Some(true)
        || explicit_type.unwrap_or_default() >= 10
    {
        "svip".to_owned()
    } else if lower_level.contains("vip")
        || lower_level.contains("绿钻")
        || lower_level.contains("豪华")
        || lower_level.contains("付费")
        || lower_level.contains("会员")
        || normal_vip == Some(true)
        || explicit_type.unwrap_or_default() > 0
    {
        "vip".to_owned()
    } else {
        "none".to_owned()
    };

    let usable_explicit_label = if !explicit_level.is_empty()
        && !matches!(
            explicit_level.to_ascii_lowercase().as_str(),
            "0" | "1" | "true" | "false" | "vip" | "svip" | "none"
        )
        && (explicit_level.to_ascii_lowercase().contains("vip")
            || explicit_level.contains("svip")
            || explicit_level.contains("绿钻")
            || explicit_level.contains("豪华")
            || explicit_level.contains("会员")
            || explicit_level.to_ascii_lowercase().contains("super"))
    {
        explicit_level
            .replace(char::is_whitespace, "")
            .replace("绿钻豪华版", "豪华绿钻")
    } else {
        String::new()
    };

    let fallback_tier = if level == "svip" {
        super_tier
            .or(normal_tier)
            .or_else(|| parse_vip_tier_from_text(&explicit_level))
    } else if level == "vip" {
        normal_tier.or_else(|| parse_vip_tier_from_text(&explicit_level))
    } else {
        None
    };
    let tier = badge_icon
        .as_ref()
        .and_then(|(_, _, tier)| *tier)
        .or(fallback_tier);
    let tier_name = vip_level_name_of(tier);
    let base_label = if !usable_explicit_label.is_empty() {
        usable_explicit_label
    } else if level == "svip" {
        "超级会员".to_owned()
    } else if level == "vip" {
        "豪华绿钻".to_owned()
    } else {
        "未开通".to_owned()
    };
    let label = append_vip_tier(&base_label, tier_name.as_deref());
    let resolved_vip_icon_url = badge_icon
        .as_ref()
        .map(|(url, _, _)| url.clone())
        .or(vip_icon_url)
        .or_else(|| qq_official_vip_icon_url(&level, tier));

    status.vip_type = Some(explicit_type.unwrap_or_else(|| {
        if level == "svip" {
            11
        } else if level == "vip" {
            1
        } else {
            0
        }
    }));
    status.vip_level = Some(level.clone());
    status.is_vip = Some(level == "vip" || level == "svip");
    status.is_svip = Some(level == "svip");
    status.vip_label = Some(label);
    status.vip_icon = if level == "svip" {
        Some("qq-super-vip".to_owned())
    } else if level == "vip" {
        Some("qq-green-vip".to_owned())
    } else {
        None
    };
    status.vip_icon_url = resolved_vip_icon_url;
    status.vip_tier = tier;
    status.vip_level_name = tier_name;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        is_favorite_playlist, is_qzone_background_playlist, map_qq_login_status,
        qq_login_avatar_url, qq_login_nickname, qq_song_url_restriction, read_playlist_list,
    };
    use crate::{providers::error::ProviderErrorCode, types::PlaylistSummary};

    #[test]
    fn read_playlist_list_supports_multiple_shapes() {
        let created = json!({
            "data": {
                "disslist": [{ "disstid": "1" }]
            }
        });
        let collected = json!({
            "list": [{ "disstid": "2" }]
        });
        assert_eq!(read_playlist_list(&created).len(), 1);
        assert_eq!(read_playlist_list(&collected).len(), 1);
    }

    #[test]
    fn playlist_flags_detect_favorites_and_qzone_background() {
        let favorite = PlaylistSummary {
            provider: "qq".to_owned(),
            id: "1".to_owned(),
            cover_url: String::new(),
            name: "我喜欢".to_owned(),
            track_count: None,
            track_ids: Vec::new(),
            collected: Some(false),
        };
        let ordinary = PlaylistSummary {
            provider: "qq".to_owned(),
            id: "2".to_owned(),
            cover_url: String::new(),
            name: "收藏歌单".to_owned(),
            track_count: None,
            track_ids: Vec::new(),
            collected: Some(false),
        };
        let qzone_raw = json!({ "hostname": "Qzone" });

        assert!(is_favorite_playlist(&favorite));
        assert!(!is_favorite_playlist(&ordinary));
        assert!(is_qzone_background_playlist(&ordinary, &qzone_raw));
    }

    #[test]
    fn qq_song_url_restriction_maps_missing_playback_key() {
        let body = json!({
            "req_0": {
                "data": {
                    "midurlinfo": [{
                        "result": 104003,
                        "msg": "no vkey"
                    }]
                }
            }
        });
        let err = qq_song_url_restriction(&body, "track-1", true, false).unwrap();
        assert!(matches!(err.code, ProviderErrorCode::LoginRequired));
        assert_eq!(err.action.as_deref(), Some("login"));
        assert_eq!(err.raw_message.as_deref(), Some("no vkey"));
    }

    #[test]
    fn qq_login_status_maps_super_vip_payload() {
        let body = json!({
            "data": {
                "mymusic": [],
                "mydiss": []
            }
        });
        let vip = json!({
            "getVipInfo": {
                "data": {
                    "infoMap": {
                        "123": {
                            "iVipFlag": 1,
                            "iSuperVip": 1,
                            "iSuperVipLevel": 5,
                            "iconUrl": "//y.qq.com/super-vip.png"
                        }
                    }
                }
            },
            "getNickHead": {
                "data": {
                    "map_userinfo": {
                        "123": {
                            "nick": "绿钻用户",
                            "headurl": "http://q.qlogo.cn/head.jpg"
                        }
                    }
                }
            }
        });
        let status = map_qq_login_status(Some(&body), Some(&vip), Some("123"));
        assert_eq!(status.nickname.as_deref(), Some("绿钻用户"));
        assert_eq!(
            status.avatar_url.as_deref(),
            Some("https://q.qlogo.cn/head.jpg")
        );
        assert_eq!(status.user_id.as_deref(), Some("123"));
        assert_eq!(status.vip_type, Some(11));
        assert_eq!(status.vip_level.as_deref(), Some("svip"));
        assert_eq!(status.is_vip, Some(true));
        assert_eq!(status.is_svip, Some(true));
        assert_eq!(status.vip_label.as_deref(), Some("超级会员·伍"));
        assert_eq!(status.vip_icon.as_deref(), Some("qq-super-vip"));
        assert_eq!(
            status.vip_icon_url.as_deref(),
            Some("https://y.qq.com/super-vip.png")
        );
        assert_eq!(status.vip_tier, Some(5));
        assert_eq!(status.vip_level_name.as_deref(), Some("伍"));
    }

    #[test]
    fn qq_login_status_uses_official_badge_icon_fallback() {
        let vip = json!({
            "getVipInfo": {
                "data": {
                    "infoMap": {
                        "123": {
                            "iNewVip": 1,
                            "iNewSuperVip": 1,
                            "iCurLevel": 6,
                            "sIcon": "placeholder"
                        }
                    }
                }
            }
        });
        let status = map_qq_login_status(None, Some(&vip), Some("123"));
        assert_eq!(status.vip_level.as_deref(), Some("svip"));
        assert_eq!(status.vip_label.as_deref(), Some("超级会员·陆"));
        assert_eq!(
            status.vip_icon_url.as_deref(),
            Some("https://y.qq.com/mediastyle/lv-icon/v14/2x/svip6.png")
        );
        assert_eq!(status.vip_tier, Some(6));
        assert_eq!(status.vip_level_name.as_deref(), Some("陆"));
    }

    #[test]
    fn qq_legacy_test_helpers_still_follow_vip_fallback_shape() {
        let vip = json!({
            "getNickHead": {
                "data": {
                    "map_userinfo": {
                        "123": {
                            "nick": "QQ昵称",
                            "headurl": "http://q.qlogo.cn/head.jpg"
                        }
                    }
                }
            }
        });

        assert_eq!(
            qq_login_nickname(None, Some(&vip), "123").as_deref(),
            Some("QQ昵称")
        );
        assert_eq!(
            qq_login_avatar_url(None, Some(&vip), "123").as_deref(),
            Some("http://q.qlogo.cn/head.jpg")
        );
    }
}
