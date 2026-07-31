use std::{collections::HashMap, sync::Arc};

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
        PlaylistSummary, ProviderId, ProviderLoginStatus, SongLikeAck, SongUrlOptions,
        SongUrlResult, Track, TrackQualityAvailability,
    },
    utils::decrypt_qrc,
};
use async_trait::async_trait;
use tokio::sync::RwLock;

use super::client::QqClient;

const QQ_QUALITY_CANDIDATES: [QqQualityCandidate; 5] = [
    QqQualityCandidate::new("RS01", ".flac", "hires"),
    QqQualityCandidate::new("F000", ".flac", "lossless"),
    QqQualityCandidate::new("M800", ".mp3", "exhigh"),
    QqQualityCandidate::new("M500", ".mp3", "standard"),
    QqQualityCandidate::new("C400", ".m4a", "aac"),
];

#[derive(Clone, Copy)]
struct QqQualityCandidate {
    prefix: &'static str,
    extension: &'static str,
    level: &'static str,
}

impl QqQualityCandidate {
    const fn new(prefix: &'static str, extension: &'static str, level: &'static str) -> Self {
        Self {
            prefix,
            extension,
            level,
        }
    }
}

#[derive(Clone)]
pub struct QqAdapter {
    client: Arc<QqClient>,
    created_playlist_dirids: Arc<RwLock<HashMap<u64, u64>>>,
    liked_playlist_dirid: Arc<RwLock<Option<u64>>>,
}

impl QqAdapter {
    pub fn new(client: Arc<QqClient>) -> Self {
        Self {
            client,
            created_playlist_dirids: Arc::new(RwLock::new(HashMap::new())),
            liked_playlist_dirid: Arc::new(RwLock::new(None)),
        }
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
                .unwrap_or_else(|| "standard".to_owned())
                .as_str(),
        );
        let media_mid = track.id.clone();
        let filename = QQ_QUALITY_CANDIDATES
            .iter()
            .find(|q| q.level == requested)
            .map(|candidate| format!("{}{}{}", candidate.prefix, media_mid, candidate.extension))
            .unwrap();
        Ok(self
            .client
            .song_url(&track.source_id, filename)
            .await?
            .standardize(requested))
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
            provider: ProviderId::Qq,
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
        let created = match self.client.user_songlists(&euin).await {
            Ok(response) => {
                *self.liked_playlist_dirid.write().await = response.liked_dirid();
                *self.created_playlist_dirids.write().await = response.tid_to_dirid();
                response.standardize()
            }
            Err(_) => None,
        };

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
        self.created_playlist_dirids.write().await.clear();
        *self.liked_playlist_dirid.write().await = None;
        auth_session::clear_runtime_provider_cookie(&ProviderId::Qq).await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> ProviderResult<SongLikeAck> {
        self.client.ensure_login().await?;
        let dirid = (*self.liked_playlist_dirid.read().await).ok_or_else(|| ProviderError {
            code: ProviderErrorCode::NoPlaylist,
            provider: ProviderId::Qq,
            message: "qq liked playlist has not been loaded".to_owned(),
            retryable: false,
            action: Some("refresh_playlists".to_owned()),
            raw_message: None,
        })?;
        let body = self
            .client
            .update_song_in_playlist(dirid, id, liked)
            .await?;
        if body.succeeded() {
            return Ok(SongLikeAck {
                provider: ProviderId::Qq,
                id: id.to_owned(),
                liked,
                code: None,
            });
        }
        Err(ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: ProviderId::Qq,
            message: "qq like-song failed".to_owned(),
            retryable: false,
            action: None,
            raw_message: None,
        })
    }

    async fn update_song_in_playlist(
        &self,
        tid: &str,
        track_id: &str,
        adding: bool,
    ) -> ProviderResult<PlaylistAddSongAck> {
        self.client.ensure_login().await?;
        let tid = tid.parse::<u64>().map_err(|_| ProviderError {
            code: ProviderErrorCode::NoPlaylist,
            provider: ProviderId::Qq,
            message: "qq playlist id must be a numeric tid".to_owned(),
            retryable: false,
            action: None,
            raw_message: None,
        })?;
        let dirid = self
            .created_playlist_dirids
            .read()
            .await
            .get(&tid)
            .copied()
            .ok_or_else(|| ProviderError {
                code: ProviderErrorCode::NoPlaylist,
                provider: ProviderId::Qq,
                message: format!("qq created playlist {tid} has not been loaded"),
                retryable: false,
                action: Some("refresh_playlists".to_owned()),
                raw_message: None,
            })?;
        let body = self
            .client
            .update_song_in_playlist(dirid, track_id, adding)
            .await?;
        if body.succeeded() {
            return Ok(PlaylistAddSongAck {
                provider: ProviderId::Qq,
                playlist_id: tid.to_string(),
                track_id: track_id.to_owned(),
                success: true,
                code: None,
            });
        }
        Err(ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: ProviderId::Qq,
            message: if adding {
                "qq add-song failed"
            } else {
                "qq del-song failed"
            }
            .to_owned(),
            retryable: false,
            action: None,
            raw_message: None,
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
