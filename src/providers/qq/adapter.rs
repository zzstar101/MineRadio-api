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
        LyricPayload, PlaylistAddSongAck, PlaylistDetail, PlaylistSummary, ProviderId,
        ProviderLoginStatus, SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    },
};

use super::{
    client::QqClient,
    map::{
        map_qq_lyric_to_payload, map_qq_playlist_to_detail, map_qq_playlist_to_summary,
        map_qq_song_to_track,
    },
};

const QQ_QUALITIES: [&str; 5] = ["flac", "ape", "320", "128", "m4a"];

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

    async fn search(&self, keyword: &str, limit: u32) -> Result<Vec<Track>> {
        let body = self.client.search(keyword, limit).await?;
        let list = body
            .get("data")
            .and_then(|value| value.get("song"))
            .and_then(|value| value.get("list"))
            .or_else(|| body.get("song").and_then(|value| value.get("list")))
            .or_else(|| body.get("list"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(list.iter().map(map_qq_song_to_track).collect())
    }

    async fn song_url(&self, track: &Track, opts: Option<SongUrlOptions>) -> Result<SongUrlResult> {
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
        let has_cookie = self
            .client
            .current_cookie()
            .await
            .map(|cookie| !cookie.trim().is_empty())
            .unwrap_or(false);
        let mut last_error = None;

        for quality in qualities {
            match self
                .client
                .song_url(&track.source_id, &media_mid, quality)
                .await
            {
                Ok(body) => {
                    let url = qq_song_url_info(&body);
                    if let Some(url) = url {
                        return Ok(SongUrlResult {
                            url: Some(url),
                            quality: Some(qq_quality_label(quality).to_owned()),
                            expires_at: None,
                        });
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
            message: last_error.unwrap_or_else(|| {
                format!("qq song-url {} returned no url", track.source_id)
            }),
            retryable: false,
            action: None,
            raw_message: None,
        })
    }

    async fn track_qualities(&self, track: &Track) -> Result<TrackQualityAvailability> {
        let body = self.client.song_detail(&track.source_id).await?;
        let file = find_file_object(&body);
        let qualities = QQ_QUALITIES
            .into_iter()
            .filter(|quality| file_supports_quality(file, quality))
            .map(str::to_owned)
            .collect();
        Ok(TrackQualityAvailability { qualities })
    }

    async fn lyric(&self, track: &Track) -> Result<LyricPayload> {
        let body = self.client.lyric(&track.source_id).await?;
        Ok(map_qq_lyric_to_payload(
            &track.source_id,
            body.get("lyric").and_then(Value::as_str).unwrap_or_default(),
            body.get("trans").and_then(Value::as_str).unwrap_or_default(),
            body.get("qrc").and_then(Value::as_str).unwrap_or_default(),
        ))
    }

    async fn playlist_list(&self) -> Result<Vec<PlaylistSummary>> {
        let cookie = self.client.current_cookie().await;
        let Some(cookie) = cookie.filter(|cookie| !cookie.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        let user_id = qq_user_id_from_cookie(&cookie);
        let Some(user_id) = user_id else {
            return Ok(Vec::new());
        };
        let created = self.client.user_songlists(&user_id).await.ok();
        let collected = self.client.user_collect_songlists(&user_id).await.ok();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();

        if let Some(created) = created {
            if let Some(items) = created
                .get("data")
                .and_then(|value| value.get("disslist"))
                .and_then(Value::as_array)
            {
                for item in items {
                    let summary = map_qq_playlist_to_summary(item, None);
                    if !summary.id.is_empty() && seen.insert(summary.id.clone()) {
                        out.push(summary);
                    }
                }
            }
        }

        if let Some(collected) = collected {
            if let Some(items) = collected
                .get("data")
                .and_then(|value| value.get("cdlist"))
                .and_then(Value::as_array)
            {
                for item in items {
                    let summary = map_qq_playlist_to_summary(item, None);
                    if !summary.id.is_empty() && seen.insert(summary.id.clone()) {
                        out.push(summary);
                    }
                }
            }
        }

        Ok(out)
    }

    async fn playlist_detail(&self, id: &str) -> Result<PlaylistDetail> {
        let body = self.client.playlist_detail(id).await?;
        let first = body
            .get("cdlist")
            .and_then(Value::as_array)
            .and_then(|items| items.first());
        if first.is_none() {
            return Err(ProviderError {
                code: ProviderErrorCode::NoPlaylist,
                provider: "qq".to_owned(),
                message: format!("qq playlist {id} missing payload"),
                retryable: false,
                action: None,
                raw_message: Some(body.to_string()),
            });
        }
        Ok(map_qq_playlist_to_detail(first, Some(id)))
    }

    async fn login_status(&self) -> Result<ProviderLoginStatus> {
        let cookie = self.client.current_cookie().await;
        let Some(cookie) = cookie.filter(|cookie| !cookie.trim().is_empty()) else {
            return Ok(ProviderLoginStatus {
                logged_in: false,
                nickname: None,
            });
        };
        let user_id = qq_user_id_from_cookie(&cookie);
        let Some(user_id) = user_id else {
            return Ok(ProviderLoginStatus {
                logged_in: true,
                nickname: None,
            });
        };

        let body = self.client.login_status(&user_id).await?;
        let nickname = body
            .get("data")
            .and_then(|value| value.get("creator"))
            .and_then(|value| value.get("nick"))
            .and_then(Value::as_str)
            .or_else(|| {
                body.get("data")
                    .and_then(|value| value.get("creator"))
                    .and_then(|value| value.get("hostname"))
                    .and_then(Value::as_str)
            })
            .map(str::to_owned);
        Ok(ProviderLoginStatus {
            logged_in: body.get("code").and_then(Value::as_i64) != Some(1000),
            nickname,
        })
    }

    async fn logout(&self) -> Result<()> {
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie("qq").await;
        Ok(())
    }

    async fn add_song_to_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> Result<PlaylistAddSongAck> {
        ensure_cookie(self.client.current_cookie().await)?;
        let body = self.client.add_song_to_playlist(playlist_id, track_id).await?;
        let code = body
            .get("result")
            .or_else(|| body.get("code"))
            .and_then(Value::as_i64)
            .unwrap_or_default();
        if matches!(code, 0 | 100) {
            return Ok(PlaylistAddSongAck {
                playlist_id: playlist_id.to_owned(),
                track_id: track_id.to_owned(),
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

fn ensure_cookie(cookie: Option<String>) -> Result<()> {
    if cookie.as_deref().map(str::trim).unwrap_or_default().is_empty() {
        return Err(ProviderError {
            code: ProviderErrorCode::LoginRequired,
            provider: "qq".to_owned(),
            message: "qq login required".to_owned(),
            retryable: true,
            action: Some("login".to_owned()),
            raw_message: None,
        });
    }
    Ok(())
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

fn find_file_object(body: &Value) -> Option<&Value> {
    if let Some(file) = body.get("songinfo").and_then(|value| value.get("data")).and_then(|value| value.get("track_info")).and_then(|value| value.get("file")) {
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

fn qq_user_id_from_cookie(cookie: &str) -> Option<String> {
    let map = cookie
        .split(';')
        .filter_map(|segment| {
            let (name, value) = segment.trim().split_once('=')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let login_type = map
        .get("login_type")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_default();
    let raw = if login_type == 2 {
        map.get("wxuin").or_else(|| map.get("uin")).or_else(|| map.get("p_uin"))
    } else {
        map.get("uin")
            .or_else(|| map.get("qqmusic_uin"))
            .or_else(|| map.get("wxuin"))
            .or_else(|| map.get("p_uin"))
    }?;
    let digits = raw.chars().filter(|ch| ch.is_ascii_digit()).collect::<String>();
    (!digits.is_empty()).then_some(digits)
}
