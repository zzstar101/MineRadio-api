use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::client::SodaClient;
use crate::parsers2::{
    MemchrParsers,
    lrc::{LrcParser, UniversalLrcParser},
    soda_music::SodaParser,
};
use crate::{
    providers::{
        ProviderAdapter, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    services::auth_session,
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlaylistDetail, PlaylistSummary, ProviderId,
        ProviderLoginStatus, SongLikeAck, SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track,
        TrackQualityAvailability,
    },
};

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
        self.client
            .song_url(&track.source_id)
            .await?
            .standardize(opts.unwrap_or_default())
            .ok_or_else(|| unavailable(format!("soda track {} missing play info", track.source_id)))
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        self.client
            .track_detail(&track.source_id)
            .await?
            .standardize_track_qualities()
            .ok_or_else(|| no_result("track_qualities"))
    }

    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload> {
        let (lyrics, trans, track_id) = self
            .client
            .lyric(&track.source_id)
            .await?
            .standardize_lyric();
        let trans = trans
            .and_then(|t| UniversalLrcParser.parse(t).ok())
            .map(|t| {
                t.into_iter()
                    .map(|line| (line.time_ms, line.text))
                    .collect::<std::collections::HashMap<_, _>>()
            });

        let (lines, has_translation) = {
            let base_lines = match SodaParser.parse(lyrics.clone()) {
                Ok(l) => l,
                Err(e) => match UniversalLrcParser.parse(lyrics) {
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
            provider: "soda".to_owned(),
            track_id,
            lines,
            has_translation,
            is_word_by_word,
        })
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        self.client.ensure_login().await?;
        self.client
            .playlist_list()
            .await?
            .standardize()
            .ok_or_else(|| no_result("playlist_list"))
    }

    async fn playlist_detail(&self, id: &str) -> ProviderResult<PlaylistDetail> {
        self.client
            .playlist_detail(id)
            .await?
            .standardize()
            .ok_or_else(|| no_result("playlist_detail"))
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
            if body.is_collected() == Some(true) {
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

fn no_result(action: &str) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::NoResult,
        provider: "soda".to_owned(),
        message: format!("{} no result", action),
        retryable: false,
        action: Some(action.to_string()),
        raw_message: None,
    }
}

fn invalid_response(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::InvalidResponse,
        provider: "soda".to_owned(),
        message,
        retryable: false,
        action: None,
        raw_message: None,
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
