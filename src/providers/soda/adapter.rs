use std::sync::Arc;

use async_trait::async_trait;

use super::{client::SodaClient, lyric::SodaParser};
use crate::providers::lyric::{LrcParser, MemchrParsers, UniversalLrcParser};
use crate::utils::single_flight::SingleFlightCache;
use crate::{
    auth_session,
    providers::{
        ProviderAdapter, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    sidecar_log,
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, SongLikeAck, SongLikeCheckAck,
        SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    },
};

/// 分页缓存扩容步长与失败重试次数(总尝试 = 1 + PAGE_RETRIES)
const PAGE_BATCH: u32 = 200;
const PAGE_RETRIES: u32 = 2;

#[derive(Clone)]
pub struct SodaAdapter {
    client: Arc<SodaClient>,
    album_cache: Arc<SingleFlightCache<AlbumDetail>>,
}

impl SodaAdapter {
    pub fn new(client: Arc<SodaClient>) -> Self {
        Self {
            client,
            album_cache: SingleFlightCache::shared(ProviderId::Soda, PAGE_BATCH, PAGE_RETRIES),
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
        let primary = self
            .client
            .search_track(keyword, offset)
            .await
            .ok()
            .and_then(|response| response.standardize_tracks())
            .filter(|tracks| !tracks.is_empty());
        let mut t = match primary {
            Some(tracks) => tracks,
            None => {
                sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                    "Soda 主搜索无返回, 回退 search_track_fallback(keyword={keyword})"
                )));
                self.client
                    .search_track_fallback(keyword, offset, limit)
                    .await?
                    .standardize()
                    .ok_or_else(|| no_result("search_track"))?
            }
        };
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
        // 专辑接口返回全量曲目表, capacity 参数用不上; 缓存整个 AlbumDetail,
        // 命中时也能带上名称/封面等元信息(旧实现只缓存曲目列表, 命中路径元信息为空)
        let client = Arc::clone(&self.client);
        let page = self
            .album_cache
            .get(id, offset, limit, move |id, _| {
                let client = Arc::clone(&client);
                async move { Ok(client.album_detail(&id).await?.standardize()) }
            })
            .await?;
        let mut value = page.value;
        value.has_more = Some(page.has_more);
        Ok(value)
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
        self.client.ensure_login().await?;
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

    async fn update_song_in_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
        adding: bool,
    ) -> ProviderResult<PlaylistAddSongAck> {
        self.client
            .update_song_in_playlist(playlist_id, track_id, adding)
            .await?;
        //经过测试无法简易判断是否成功, 乱输一个歌曲id和正常操作响应体无异
        Ok(PlaylistAddSongAck {
            provider: ProviderId::Soda,
            playlist_id: playlist_id.to_string(),
            track_id: track_id.to_string(),
            success: true,
            code: None,
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
