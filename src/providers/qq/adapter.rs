use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    parsers::{
        lrc::{LrcParser, UniversalLrcParser},
        qqmusic::QQMusicParser,
    },
    providers::{
        ProviderAdapter, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    services::auth_session,
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, SongUrlOptions, SongUrlResult, Track,
        TrackQualityAvailability,
    },
    utils::decrypt_qrc,
};
use async_trait::async_trait;
use serde_json::Value;

use super::client::QqClient;

const QQ_QUALITY_CANDIDATES: [QqQualityCandidate; 5] = [
    QqQualityCandidate::new("RS01", ".flac", "hires", "Hi-Res FLAC"),
    QqQualityCandidate::new("F000", ".flac", "lossless", "FLAC"),
    QqQualityCandidate::new("M800", ".mp3", "exhigh", "320k MP3"),
    QqQualityCandidate::new("M500", ".mp3", "standard", "128k MP3"),
    QqQualityCandidate::new("C400", ".m4a", "aac", "AAC/M4A"),
];
const QQ_AUDIO_PROBE_TOTAL_TIMEOUT: Duration = Duration::from_millis(6200);
const QQ_AUDIO_PROBE_ATTEMPT_TIMEOUT: Duration = Duration::from_millis(2000);

#[derive(Clone, Copy)]
struct QqQualityCandidate {
    prefix: &'static str,
    extension: &'static str,
    level: &'static str,
    label: &'static str,
}

impl QqQualityCandidate {
    const fn new(
        prefix: &'static str,
        extension: &'static str,
        level: &'static str,
        label: &'static str,
    ) -> Self {
        Self {
            prefix,
            extension,
            level,
            label,
        }
    }
}

struct QqPlaybackUrl {
    filename: String,
    url: String,
}

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
        ProviderId::Qq
    }

    async fn search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<Track>> {
        let tracks = self
            .client
            .search(keyword, offset, limit)
            .await?
            .standardize();
        if !tracks.is_empty() {
            return Ok(tracks);
        }

        Ok(self
            .client
            .multi_search_track(keyword, offset, limit)
            .await?
            .standardize_songs()
            .unwrap_or_default())
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
            .standardize_albums()
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
            .standardize_playlists()
            .unwrap_or_default())
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
        let media_mids = qq_media_mids(track);
        let candidates = qq_filename_candidates(&media_mids, &requested);
        let filenames = candidates
            .iter()
            .map(|candidate| candidate.filename.clone())
            .collect::<Vec<_>>();
        let cookie = self.client.current_cookie().await.unwrap_or_default();
        let has_cookie = !cookie.trim().is_empty();
        let has_playback_key = QqClient::has_playback_key(&cookie);
        let last_error = match self.client.song_url(&track.source_id, &filenames).await {
            Ok(body) => {
                let deadline = Instant::now() + QQ_AUDIO_PROBE_TOTAL_TIMEOUT;
                for playback in qq_song_url_candidates(&body) {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining < Duration::from_millis(300) {
                        break;
                    }
                    if self
                        .client
                        .probe_playback_url(
                            &playback.url,
                            remaining.min(QQ_AUDIO_PROBE_ATTEMPT_TIMEOUT),
                        )
                        .await
                    {
                        let candidate = candidates
                            .iter()
                            .find(|candidate| candidate.filename == playback.filename);
                        return Ok(SongUrlResult {
                            url: Some(playback.url),
                            proxied: false,
                            provider: Some(ProviderId::Qq),
                            trial: Some(false),
                            playable: Some(true),
                            level: Some(
                                candidate
                                    .map(|candidate| candidate.level)
                                    .unwrap_or(playback.filename.as_str())
                                    .to_owned(),
                            ),
                            quality: Some(
                                candidate
                                    .map(|candidate| candidate.label)
                                    .unwrap_or("QQ")
                                    .to_owned(),
                            ),
                            filename: Some(playback.filename),
                            requested_quality: Some(requested.clone()),
                            expires_at: None,
                            ..Default::default()
                        });
                    }
                }
                if let Some(error) =
                    qq_song_url_restriction(&body, &track.source_id, has_cookie, has_playback_key)
                {
                    return Err(error);
                }
                "qq song-url returned no verified playback url".to_owned()
            }
            Err(err) => err.message,
        };

        if !has_cookie {
            return Err(ProviderError {
                code: ProviderErrorCode::LoginRequired,
                provider: ProviderId::Qq,
                message: format!("qq song-url {} requires cookie", track.source_id),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: None,
            });
        }

        Err(ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: ProviderId::Qq,
            message: last_error,
            retryable: false,
            action: None,
            raw_message: None,
        })
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        self.client
            .song_detail(&track.source_id)
            .await?
            .standardize()
            .ok_or_else(|| no_result("track_qualities"))
    }

    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload> {
        let (lyrics, trans) = self.client.lyric(&track.source_id).await?.standardize();
        // 温馨提示, 两个都是加了密的
        // TODO: 由于翻译歌词使用百分秒结构所以hashmap生成的和逐字歌词可能出现不吻合导致无法将对应翻译句子放入正确部分
        let lyric = lyrics.ok_or_else(|| no_result("lyric"))?;
        let trans = trans
            .and_then(|t| decrypt_qrc(&t).ok())
            .and_then(|t| UniversalLrcParser.parse(t).ok())
            .map(|t| {
                t.into_iter()
                    .map(|line| (line.time_ms, line.text))
                    .collect::<std::collections::HashMap<_, _>>()
            });
        let (lines, has_translation) = {
            let base_lines = match QQMusicParser.decrypt_and_parse(lyric.clone()) {
                Ok(l) => l,
                Err(e) => match UniversalLrcParser.parse(lyric) {
                    Ok(l) => l,
                    Err(e2) => return Err(invalid_response(e + " " + &e2)),
                },
            };
            match trans {
                Some(trans) => (
                    base_lines
                        .into_iter()
                        .map(|mut line| {
                            line.translation = trans
                                .get(&line.time_ms)
                                .cloned()
                                .filter(|value| !value.is_empty());
                            line
                        })
                        .collect::<Vec<_>>(),
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
            provider: ProviderId::Soda,
            track_id: track.id.clone(),
            lines,
            has_translation,
            is_word_by_word,
        })
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        self.client.ensure_login().await?;
        let euin = self.client.euin().await.unwrap_or_default();
        let uin = self.client.uin().await.unwrap_or_default();
        let created = self
            .client
            .user_songlists(&euin)
            .await
            .ok()
            .and_then(|l| l.standardize());

        let collected = self
            .client
            .user_collect_songlists(&uin)
            .await
            .ok()
            .and_then(|l| l.standardize());

        let mut out = match created {
            Some(v) => v,
            None => Vec::new(),
        };

        if let Some(collected) = collected {
            out.extend(collected);
        }

        Ok(out)
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
        Ok(self.client.album_list().await?.standardize())
    }

    async fn album_detail(&self, id: &str, offset: u32, limit: u32) -> ProviderResult<AlbumDetail> {
        Ok(self
            .client
            .album_detail(id, offset, limit)
            .await?
            .standardize())
    }

    async fn login_status(&self) -> ProviderResult<ProviderLoginStatus> {
        let cookie = self.client.current_cookie().await;
        let Some(cookie) = cookie.filter(|cookie| !cookie.trim().is_empty()) else {
            return Ok(qq_logged_out_status());
        };
        let euin = self.client.euin().await;
        let Some(euin) = euin else {
            return Ok(qq_logged_out_status());
        };
        match tokio::try_join!(
            self.client.login_status_with_cookie(&euin, &cookie),
            self.client.vip_info_with_cookie(&euin, &cookie),
        ) {
            Ok((login_status, vip_info)) => Ok(login_status.standardize(vip_info)),
            Err(_) => Ok(qq_logged_out_status()),
        }
    }

    async fn logout(&self) -> ProviderResult<()> {
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie(&ProviderId::Qq).await;
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
                provider: ProviderId::Qq,
                playlist_id: playlist_id.to_owned(),
                track_id: track_id.to_owned(),
                success: true,
                code: Some(code),
            });
        }
        if matches!(code, 301 | 1000) {
            return Err(ProviderError {
                code: ProviderErrorCode::LoginRequired,
                provider: ProviderId::Qq,
                message: format!("qq playlist {playlist_id} add-song requires cookie"),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: Some(body.to_string()),
            });
        }
        Err(ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: ProviderId::Qq,
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
        "jymaster" | "master" | "studio" | "svip" => "hires".to_owned(),
        "hires" | "hi-res" | "highres" | "zhenyin" | "spatial" => "hires".to_owned(),
        "lossless" | "flac" | "sq" => "lossless".to_owned(),
        "exhigh" | "high" | "320" | "320k" | "hq" => "exhigh".to_owned(),
        "standard" | "normal" | "128" | "128k" | "std" => "standard".to_owned(),
        "aac" | "m4a" => "aac".to_owned(),
        _ => "hires".to_owned(),
    }
}

fn qq_filename_candidates(media_mids: &[String], requested: &str) -> Vec<QqFilenameCandidate> {
    let start = QQ_QUALITY_CANDIDATES
        .iter()
        .position(|candidate| candidate.level == requested)
        .unwrap_or(0);
    media_mids
        .iter()
        .flat_map(|media_mid| {
            QQ_QUALITY_CANDIDATES[start..]
                .iter()
                .map(move |candidate| QqFilenameCandidate {
                    filename: format!("{}{}{}", candidate.prefix, media_mid, candidate.extension),
                    level: candidate.level,
                    label: candidate.label,
                })
        })
        .collect()
}

struct QqFilenameCandidate {
    filename: String,
    level: &'static str,
    label: &'static str,
}

fn qq_media_mids(track: &Track) -> Vec<String> {
    let mut ids = Vec::with_capacity(2);
    if let Some(media_mid) = track
        .media_mid
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        ids.push(media_mid.to_owned());
    }
    if !track.source_id.trim().is_empty() && !ids.iter().any(|id| id == &track.source_id) {
        ids.push(track.source_id.clone());
    }
    ids
}

fn qq_song_url_candidates(body: &Value) -> Vec<QqPlaybackUrl> {
    let data = body
        .get("req_0")
        .and_then(|value| value.get("data"))
        .or_else(|| body.get("data"));
    let Some(data) = data else {
        return Vec::new();
    };
    let sips = data
        .get("sip")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec!["https://ws.stream.qqmusic.qq.com/"]);
    data.get("midurlinfo")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|info| {
            let purl = info.get("purl").and_then(Value::as_str)?.trim();
            (!purl.is_empty()).then_some((
                info.get("filename")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                purl,
            ))
        })
        .flat_map(|(filename, purl)| {
            sips.iter().map(move |sip| QqPlaybackUrl {
                filename: filename.clone(),
                url: if purl.starts_with("http://") || purl.starts_with("https://") {
                    purl.to_owned()
                } else {
                    format!("{sip}{purl}")
                },
            })
        })
        .collect()
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
            provider: ProviderId::Qq,
            message: format!("qq song-url {track_id} requires cookie"),
            retryable: true,
            action: Some("login".to_owned()),
            raw_message,
        });
    }

    if code == 104003 && !has_playback_key {
        return Some(ProviderError {
            code: ProviderErrorCode::LoginRequired,
            provider: ProviderId::Qq,
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
            provider: ProviderId::Qq,
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
            provider: ProviderId::Qq,
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

fn no_result(action: &str) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::NoResult,
        provider: ProviderId::Qq,
        message: format!("{} no result", action),
        retryable: false,
        action: Some(action.to_string()),
        raw_message: None,
    }
}

fn qq_logged_out_status() -> ProviderLoginStatus {
    ProviderLoginStatus {
        provider: ProviderId::Qq,
        logged_in: false,
        ..Default::default()
    }
}

fn invalid_response(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::InvalidResponse,
        provider: ProviderId::Qq,
        message,
        retryable: false,
        action: None,
        raw_message: None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        qq_filename_candidates, qq_login_avatar_url, qq_login_nickname, qq_song_url_candidates,
        qq_song_url_restriction,
    };
    use crate::providers::error::ProviderErrorCode;

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
    fn qq_filename_candidates_try_media_mid_before_song_mid() {
        let candidates =
            qq_filename_candidates(&["media-mid".to_owned(), "song-mid".to_owned()], "hires");
        let filenames = candidates
            .into_iter()
            .map(|candidate| candidate.filename)
            .collect::<Vec<_>>();

        assert_eq!(
            filenames,
            vec![
                "RS01media-mid.flac",
                "F000media-mid.flac",
                "M800media-mid.mp3",
                "M500media-mid.mp3",
                "C400media-mid.m4a",
                "RS01song-mid.flac",
                "F000song-mid.flac",
                "M800song-mid.mp3",
                "M500song-mid.mp3",
                "C400song-mid.m4a",
            ]
        );
    }

    #[test]
    fn qq_song_url_candidates_try_every_sip_in_response_order() {
        let body = json!({
            "req_0": {
                "data": {
                    "sip": ["https://first/", "https://second/"],
                    "midurlinfo": [
                        {"filename": "F000one.flac", "purl": "one"},
                        {"filename": "M800two.mp3", "purl": "two"}
                    ]
                }
            }
        });
        let urls = qq_song_url_candidates(&body)
            .into_iter()
            .map(|candidate| (candidate.filename, candidate.url))
            .collect::<Vec<_>>();

        assert_eq!(
            urls,
            vec![
                ("F000one.flac".to_owned(), "https://first/one".to_owned()),
                ("F000one.flac".to_owned(), "https://second/one".to_owned()),
                ("M800two.mp3".to_owned(), "https://first/two".to_owned()),
                ("M800two.mp3".to_owned(), "https://second/two".to_owned()),
            ]
        );
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
