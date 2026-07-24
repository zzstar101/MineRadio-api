use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::client::SodaClient;
use crate::parsers::{
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

#[derive(Clone)]
pub struct SodaAdapter {
    client: Arc<SodaClient>,
    album_cache: Arc<Mutex<HashMap<String, (Vec<Track>, Instant)>>>,
}

impl SodaAdapter {
    pub fn new(client: Arc<SodaClient>) -> Self {
        Self {
            client,
            album_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new(Arc::new(SodaClient::new())))
    }
}

#[async_trait]
impl ProviderAdapter for SodaAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Soda
    }

    async fn search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<Track>> {
        let mut t = self
            .client
            .search_track(keyword, offset)
            .await?
            .standardize_tracks()
            .ok_or_else(|| no_result("search_track"))?;
        t.truncate(limit as usize);
        Ok(t)
    }

    async fn search_album(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<AlbumSummary>> {
        let mut a = self
            .client
            .search_album(keyword, offset)
            .await?
            .standardize_albums()
            .ok_or_else(|| no_result("search_album"))?;
        a.truncate(limit as usize);
        Ok(a)
    }

    async fn search_playlist(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<PlaylistSummary>> {
        let mut p = self
            .client
            .search_playlist(keyword, offset)
            .await?
            .standardize_playlists()
            .ok_or_else(|| no_result("search_playlist"))?;
        p.truncate(limit as usize);
        Ok(p)
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
            provider: ProviderId::Soda,
            track_id,
            lines,
            has_translation,
            is_word_by_word,
        })
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        self.client.ensure_login().await?;
        let mut created = self
            .client
            .user_playlist_list()
            .await?
            .standardize()
            .unwrap_or_default();
        let collected = self
            .client
            .user_collected_list()
            .await?
            .standardize_playlists()
            .unwrap_or_default();
        created.extend(collected);
        if created.is_empty() {
            Err(no_result("playlist_list"))
        } else {
            Ok(created)
        }
    }

    async fn playlist_detail(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<PlaylistDetail> {
        self.client
            .playlist_detail(id, offset, limit)
            .await?
            .standardize()
            .ok_or_else(|| no_result("playlist_detail"))
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        self.client.ensure_login().await?;
        self.client
            .user_collected_list()
            .await?
            .standardize_albums()
            .ok_or_else(|| no_result("playlist_detail"))
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
                        provider: ProviderId::Soda,
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
                ProviderId::Soda,
                "no-session",
            ));
        }
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie(&ProviderId::Soda).await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> ProviderResult<SongLikeAck> {
        self.client.ensure_login().await?;
        let clean_id = id.trim();
        let req = self.client.like_song(clean_id, liked).await?;
        if req.check() {
            Ok(SongLikeAck {
                provider: ProviderId::Soda,
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
            provider: ProviderId::Soda,
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
        provider: ProviderId::Soda,
        message,
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn no_result(action: &str) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::NoResult,
        provider: ProviderId::Soda,
        message: format!("{} no result", action),
        retryable: false,
        action: Some(action.to_string()),
        raw_message: None,
    }
}

fn invalid_response(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::InvalidResponse,
        provider: ProviderId::Soda,
        message,
        retryable: false,
        action: None,
        raw_message: None,
    }
}
