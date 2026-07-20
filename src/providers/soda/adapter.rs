use std::sync::Arc;

use async_trait::async_trait;

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
        let lyrics = lyrics.ok_or_else(|| no_result("lyric"))?;
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
        self.client.ensure_login().await?;
        self.client
            .login_status()
            .await?
            .standardize()
            .ok_or_else(|| no_result("login_status"))
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
        let req = self.client.like_song(clean_id, liked).await?;
        if req.check() {
            Ok(SongLikeAck {
                provider: "soda".to_owned(),
                id: clean_id.to_owned(),
                liked,
                code: Some(200),
            })
        } else {
            let (code, raw_message) = req.get_err_message();
            let message =
                format!("soda like_song failed with code {code}, raw_message: {raw_message}");
            Err(unavailable(message))
        }
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
        let liked_set = liked_ids
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>();

        Ok(SongLikeCheckAck {
            provider: "soda".to_owned(),
            ids: ids.to_vec(),
            liked: ids
                .iter()
                .map(|id| (id.clone(), liked_set.contains(id)))
                .collect(),
        })
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
