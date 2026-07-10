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
        ProviderLoginStatus, SongLikeAck, SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track,
        TrackQualityAvailability,
    },
};

use super::{
    client::NeteaseClient,
    map::{
        map_hana_lyric_to_payload, map_hana_playlist_to_detail, map_hana_playlist_to_summary,
        map_hana_song_to_track, map_playable,
    },
};

const QUALITY_CANDIDATES: [(&str, u32); 9] = [
    ("jymaster", 1_999_000),
    ("dolby", 1_999_000),
    ("sky", 1_999_000),
    ("jyeffect", 1_999_000),
    ("hires", 1_999_000),
    ("lossless", 1_411_000),
    ("exhigh", 999_000),
    ("higher", 192_000),
    ("standard", 128_000),
];

#[derive(Clone, Default)]
pub struct NeteaseAdapter {
    client: Arc<NeteaseClient>,
}

impl NeteaseAdapter {
    pub fn new(client: Arc<NeteaseClient>) -> Self {
        Self { client }
    }

    async fn login_status_internal(&self) -> Result<ProviderLoginStatus> {
        let Some(cookie) = self.client.current_cookie().await else {
            return Ok(ProviderLoginStatus {
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
            });
        };
        if cookie.trim().is_empty() {
            return Ok(ProviderLoginStatus {
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
            });
        }

        let body = self.client.login_status().await?;
        let profile = body
            .get("profile")
            .or_else(|| body.get("data").and_then(|data| data.get("profile")));

        Ok(ProviderLoginStatus {
            logged_in: profile.is_some(),
            nickname: profile
                .and_then(|value| value.get("nickname"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            user_id: profile
                .and_then(|value| value.get("userId"))
                .map(read_id_like)
                .filter(|value| !value.is_empty()),
            avatar_url: profile
                .and_then(|value| value.get("avatarUrl"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .filter(|value| !value.is_empty()),
        })
    }
}

#[async_trait]
impl ProviderAdapter for NeteaseAdapter {
    fn id(&self) -> ProviderId {
        "netease".to_owned()
    }

    async fn search(&self, keyword: &str, limit: u32) -> Result<Vec<Track>> {
        let body = self.client.cloudsearch(keyword, limit).await?;
        let songs = body
            .get("result")
            .and_then(|value| value.get("songs"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        Ok(songs.iter().map(map_hana_song_to_track).collect())
    }

    async fn song_url(&self, track: &Track, opts: Option<SongUrlOptions>) -> Result<SongUrlResult> {
        let requested = opts
            .and_then(|value| value.quality)
            .unwrap_or_else(|| "hires".to_owned());
        let start_index = QUALITY_CANDIDATES
            .iter()
            .position(|(level, _)| *level == requested)
            .unwrap_or(4);
        let has_cookie = self.client.current_cookie().await.is_some();
        let mut last_state = "unknown".to_owned();

        for (level, br) in QUALITY_CANDIDATES.iter().skip(start_index) {
            let body = match self.client.song_url_v1(&track.source_id, level).await {
                Ok(body) => body,
                Err(_) => self.client.song_url(&track.source_id, *br).await?,
            };
            let datum = body
                .get("data")
                .and_then(Value::as_array)
                .and_then(|items| {
                    items.iter().find(|item| {
                        item.get("id")
                            .map(read_id_like)
                            .unwrap_or_default()
                            == track.source_id
                    })
                    .or_else(|| items.first())
                });

            let Some(datum) = datum else {
                continue;
            };
            let url = datum.get("url").and_then(Value::as_str);
            let state = map_playable(
                datum.get("fee").and_then(Value::as_i64),
                datum.get("code").and_then(Value::as_i64),
                datum.get("freeTrialInfo"),
                has_cookie,
                url,
            );
            last_state = state.clone();
            if state != "playable" {
                continue;
            }
            return Ok(SongUrlResult {
                url: url.map(str::to_owned),
                quality: datum
                    .get("level")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| Some((*level).to_owned())),
                expires_at: None,
            });
        }

        Err(state_error(&last_state, &track.source_id))
    }

    async fn track_qualities(&self, track: &Track) -> Result<TrackQualityAvailability> {
        let has_cookie = self.client.current_cookie().await.is_some();
        let mut qualities = Vec::new();

        for (level, br) in QUALITY_CANDIDATES {
            let body = match self.client.song_url_v1(&track.source_id, level).await {
                Ok(body) => body,
                Err(_) => match self.client.song_url(&track.source_id, br).await {
                    Ok(body) => body,
                    Err(_) => continue,
                },
            };
            if body.is_null() {
                continue;
            }
            let datum = body
                .get("data")
                .and_then(Value::as_array)
                .and_then(|items| items.first());
            let Some(datum) = datum else {
                continue;
            };
            let url = datum.get("url").and_then(Value::as_str);
            let state = map_playable(
                datum.get("fee").and_then(Value::as_i64),
                datum.get("code").and_then(Value::as_i64),
                datum.get("freeTrialInfo"),
                has_cookie,
                url,
            );
            if state == "playable" {
                qualities.push(level.to_owned());
            }
        }

        qualities.dedup();
        Ok(TrackQualityAvailability { qualities })
    }

    async fn lyric(&self, track: &Track) -> Result<LyricPayload> {
        let body = match self.client.lyric_new(&track.source_id).await {
            Ok(body) => body,
            Err(_) => self.client.lyric(&track.source_id).await?,
        };
        Ok(map_hana_lyric_to_payload(
            &track.source_id,
            body.get("lrc")
                .and_then(|value| value.get("lyric"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
            body.get("tlyric")
                .and_then(|value| value.get("lyric"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
            body.get("klyric")
                .and_then(|value| value.get("lyric"))
                .and_then(Value::as_str),
            body.get("yrc")
                .and_then(|value| value.get("lyric"))
                .and_then(Value::as_str),
        ))
    }

    async fn playlist_list(&self) -> Result<Vec<PlaylistSummary>> {
        ensure_logged_in(self.client.current_cookie().await)?;
        let status_body = self.client.login_status().await?;
        let profile = status_body
            .get("profile")
            .or_else(|| status_body.get("data").and_then(|data| data.get("profile")));
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
                items.iter()
                    .map(|item| map_hana_playlist_to_summary(item, None))
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn playlist_detail(&self, id: &str) -> Result<PlaylistDetail> {
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

    async fn login_status(&self) -> Result<ProviderLoginStatus> {
        self.login_status_internal().await
    }

    async fn logout(&self) -> Result<()> {
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie("netease").await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> Result<SongLikeAck> {
        ensure_logged_in(self.client.current_cookie().await)?;
        self.client.like(id, liked).await?;
        Ok(SongLikeAck {
            id: id.to_owned(),
            liked,
        })
    }

    async fn check_song_likes(&self, ids: &[String]) -> Result<SongLikeCheckAck> {
        ensure_logged_in(self.client.current_cookie().await)?;
        if ids.is_empty() {
            return Ok(SongLikeCheckAck {
                liked_ids: Vec::new(),
            });
        }

        let status_body = self.client.login_status().await?;
        let uid = status_body
            .get("profile")
            .or_else(|| status_body.get("data").and_then(|data| data.get("profile")))
            .and_then(|profile| profile.get("userId"))
            .map(read_id_like)
            .unwrap_or_default();

        let liked_ids = match self.client.song_like_check(ids).await {
            Ok(body) => body
                .get("ids")
                .or_else(|| body.get("data"))
                .and_then(Value::as_array)
                .map(|items| items.iter().map(read_id_like).collect::<Vec<_>>())
                .filter(|items| !items.is_empty())
                .unwrap_or_else(|| Vec::new()),
            Err(_) => Vec::new(),
        };

        if !liked_ids.is_empty() {
            return Ok(SongLikeCheckAck { liked_ids });
        }

        let body = self.client.likelist(&uid).await?;
        Ok(SongLikeCheckAck {
            liked_ids: body
                .get("ids")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(read_id_like).collect())
                .unwrap_or_default(),
        })
    }

    async fn add_song_to_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> Result<PlaylistAddSongAck> {
        ensure_logged_in(self.client.current_cookie().await)?;
        let primary = self.client.playlist_tracks(playlist_id, track_id).await;
        if primary.is_err() {
            self.client.playlist_track_add(playlist_id, track_id).await?;
        }
        Ok(PlaylistAddSongAck {
            playlist_id: playlist_id.to_owned(),
            track_id: track_id.to_owned(),
        })
    }
}

fn ensure_logged_in(cookie: Option<String>) -> Result<()> {
    if cookie.as_deref().map(str::trim).unwrap_or_default().is_empty() {
        return Err(ProviderError {
            code: ProviderErrorCode::LoginRequired,
            provider: "netease".to_owned(),
            message: "netease login required".to_owned(),
            retryable: true,
            action: Some("login".to_owned()),
            raw_message: None,
        });
    }
    Ok(())
}

fn read_id_like(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}

fn state_error(state: &str, id: &str) -> ProviderError {
    let code = match state {
        "login_required" => ProviderErrorCode::LoginRequired,
        "vip_required" => ProviderErrorCode::VipRequired,
        "paid_required" => ProviderErrorCode::PaidRequired,
        "trial_only" => ProviderErrorCode::TrialOnly,
        "copyright_unavailable" => ProviderErrorCode::CopyrightUnavailable,
        _ => ProviderErrorCode::Unavailable,
    };
    ProviderError {
        code,
        provider: "netease".to_owned(),
        message: format!("netease song-url {id} state {state}"),
        retryable: state == "login_required",
        action: (state == "login_required").then(|| "login".to_owned()),
        raw_message: None,
    }
}
