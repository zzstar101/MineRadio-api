use std::{collections::HashMap, sync::Arc};

use crate::{
    auth_session,
    cache::{self, TTL_1_DAY},
    providers::{
        ProviderAdapter, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
        lyric::{LrcParser, UniversalLrcParser},
    },
    sidecar_log,
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, RecommendationPage, SongLikeAck,
        SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    },
    utils::{
        cryptors::qq::x4_fix_identity,
        decrypt_qrc,
        pop_queue::{DEFAULT_LOW_WATER, DEFAULT_RETRIES, PopQueue},
    },
};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::RwLock;

use super::{client::QqClient, lyric::QQMusicParser};

const QQ_PLAIN_QUALITY_CANDIDATES: [QqQualityCandidate; 9] = [
    QqQualityCandidate::new("Q000", ".flac", "atmos"),
    QqQualityCandidate::new("O800", ".ogg", "premium"),
    QqQualityCandidate::new("AI00", ".flac", "master"),
    QqQualityCandidate::new("RS01", ".flac", "hires"),
    QqQualityCandidate::new("F000", ".flac", "flac"),
    QqQualityCandidate::new("M800", ".mp3", "320k"),
    QqQualityCandidate::new("TL01", ".nac", "nac"),
    QqQualityCandidate::new("M500", ".mp3", "128k"),
    QqQualityCandidate::new("C400", ".m4a", "aac"),
];

const QQ_ENCRYPTED_QUALITY_CANDIDATES: [QqQualityCandidate; 8] = [
    QqQualityCandidate::new("Q0M0", ".mflac", "atmos"),
    QqQualityCandidate::new("O8M0", ".mgg", "premium"),
    QqQualityCandidate::new("AIM0", ".mflac", "master"),
    QqQualityCandidate::new("RSM1", ".mflac", "hires"),
    QqQualityCandidate::new("F0M0", ".mflac", "flac"),
    QqQualityCandidate::new("O6M0", ".mgg", "320k"),
    QqQualityCandidate::new("TLM1", ".mnac", "nac"),
    QqQualityCandidate::new("O4M0", ".mgg", "128k"),
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

    fn filename(&self, media_mid: &str) -> String {
        format!("{}{}{}", self.prefix, media_mid, self.extension)
    }
}

fn qq_quality_candidates(encrypted: bool) -> &'static [QqQualityCandidate] {
    if encrypted {
        &QQ_ENCRYPTED_QUALITY_CANDIDATES
    } else {
        &QQ_PLAIN_QUALITY_CANDIDATES
    }
}

fn qq_filenames(encrypted: bool, media_mid: &str) -> Vec<String> {
    qq_quality_candidates(encrypted)
        .iter()
        .map(|candidate| candidate.filename(media_mid))
        .collect()
}

/// 雷达电台的固定流 ID(stream_next 分发用)
const RADAR_ID: &str = "22000";
/// 推荐页混合模块电台卡的固定 ID(不返回封面与标题)
const MIXED_RADIO_ID: &str = "99";

#[derive(Clone)]
pub struct QqAdapter {
    client: Arc<QqClient>,
    created_playlist_dirids: Arc<RwLock<HashMap<u64, u64>>>,
    liked_playlist_dirid: Arc<RwLock<Option<u64>>>,
    radio_queue: Arc<PopQueue<Track>>,
}

impl QqAdapter {
    pub fn new(client: Arc<QqClient>) -> Self {
        Self {
            client,
            created_playlist_dirids: Arc::new(RwLock::new(HashMap::new())),
            liked_playlist_dirid: Arc::new(RwLock::new(None)),
            radio_queue: Arc::new(PopQueue::new(
                ProviderId::Qq,
                DEFAULT_LOW_WATER,
                DEFAULT_RETRIES,
            )),
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
        let requested = opts
            .and_then(|o| o.quality)
            .unwrap_or_else(|| "128k".to_owned());
        let media_mid = track.id.clone();
        let (cdn_res, detail_res) =
            tokio::join!(self.client.cdn(), self.client.song_detail(&track.source_id));
        let cdn = cdn_res?;
        let preview_range = detail_res
            .ok()
            .and_then(|detail| detail.standardize_preview_range());

        // 普通 → 加密 → 试听; 每组一次请求带全部候选文件名, 组内按高品质优先取非空 purl
        let plain = qq_filenames(false, &media_mid);
        if let Some(r) = self
            .client
            .song_url(&track.source_id, plain.clone(), false)
            .await?
            .standardize(&cdn, false, preview_range.clone(), &plain)
        {
            return Ok(r);
        }

        let encrypted = qq_filenames(true, &media_mid);
        if let Some(r) = self
            .client
            .song_url(&track.source_id, encrypted.clone(), true)
            .await?
            .standardize(&cdn, true, preview_range.clone(), &encrypted)
        {
            return Ok(r);
        }

        // 试听固定 RS02 前缀, mid 用 songmid 本身(原生客户端级别8即如此)
        let trial = vec![format!("RS02{}.mp3", track.source_id)];
        if let Some(r) = self
            .client
            .song_url(&track.source_id, trial.clone(), false)
            .await?
            .standardize(&cdn, false, preview_range, &trial)
        {
            return Ok(r);
        }

        self.client.ensure_login().await?;
        Err(ProviderError {
            code: ProviderErrorCode::NoUrl,
            provider: ProviderId::Qq,
            message: format!("qq did not return a playable URL for {requested}"),
            retryable: false,
            action: None,
            raw_message: None,
        })
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        self.client
            .song_detail(&track.source_id)
            .await?
            .standardize_track_qualities()
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
            Err(err) => {
                sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                    "QQ 歌单列表获取失败, 回退仅展示收藏: {err}"
                )));
                None
            }
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
            .standardize(offset))
    }

    async fn stream_next(&self, id: &str) -> ProviderResult<Track> {
        let client = Arc::clone(&self.client);
        self.radio_queue
            .pop(id, move |_, want| {
                let client = Arc::clone(&client);
                let id = id.to_owned();
                async move { pull_stream_batch(&client, &id, want).await }
            })
            .await
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
        let Some(cookie) = self.client.current_cookie().await else {
            return Ok(qq_logged_out_status());
        };
        match self.client.refresh_login_cookie(cookie).await {
            Ok(Some(refreshed)) => {
                auth_session::set_runtime_provider_cookie(ProviderId::Qq, refreshed)
                    .await
                    .map_err(invalid_response)?
            }
            Err(e) => {
                sidecar_log::log_runtime(json!(format!("qq 登录换票失败: {}", e.message))).await
            }
            _ => (),
        }
        let uin = self.client.uin().await;
        let Some(uin) = uin else {
            return Ok(qq_logged_out_status());
        };
        if let Some(guid) = self.client.guid().await {
            x4_fix_identity(&uin, &guid);
        }
        let (login_status, vip_info) = tokio::join!(
            self.client.login_status_with_cookie(&uin),
            self.client.vip_info_with_cookie(&uin),
        );
        Ok(login_status?.standardize(vip_info.ok()))
    }

    async fn logout(&self) -> ProviderResult<()> {
        self.client.logout().await?;
        self.created_playlist_dirids.write().await.clear();
        *self.liked_playlist_dirid.write().await = None;
        self.radio_queue.clear();
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

    async fn recommendation_page(&self, refresh: bool) -> ProviderResult<RecommendationPage> {
        let fetch = || async {
            let response = self.client.recommend_page().await?;
            let mut track_ids = response.track_ids();
            track_ids.sort_unstable();
            track_ids.dedup();

            let mid_by_id = if track_ids.is_empty() {
                None
            } else {
                self.client
                    .get_track_info_by_ids(track_ids)
                    .await
                    .ok()
                    .and_then(|i| i.standardize())
            };
            let mut page = response
                .standardize(mid_by_id.as_ref())
                .ok_or_else(|| no_result("recommendation_page"))?;
            // 特判: 首张电台卡借队列预热补齐封面与标题, 失败不影响整页返回
            if let Err(err) = self.patch_radio_card(&mut page).await {
                sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                    "推荐页电台特判失败, 保留原样: {err}"
                )));
            }
            Ok(serde_json::to_string(&page).ok())
        };
        let raw = if refresh {
            let raw = fetch().await?;
            if let Some(value) = raw.as_ref() {
                cache::insert(
                    ProviderId::Qq,
                    "recommendation_page",
                    TTL_1_DAY,
                    value.clone(),
                )
                .await;
            }
            raw
        } else {
            cache::get_or_refresh(ProviderId::Qq, "recommendation_page", TTL_1_DAY, fetch).await?
        };
        raw.and_then(|raw| serde_json::from_str(&raw).ok())
            .ok_or_else(|| no_result("recommendation_page"))
    }
}

impl QqAdapter {
    /// 特判: 模块一首卡缺封面/缺ID/是 99 号混合电台时, 经队列 peek 预热并借首曲补齐封面与标题;
    /// 客户端随后真实播放走 stream_next 的 pop, 消费的正是这里预热的同一批
    async fn patch_radio_card(&self, page: &mut RecommendationPage) -> ProviderResult<()> {
        let Some(card) = page
            .list
            .first_mut()
            .and_then(|module| module.list.first_mut())
        else {
            return Ok(());
        };
        if !(card.cover_url.trim().is_empty() || card.id.is_empty() || card.id == MIXED_RADIO_ID) {
            return Ok(());
        }
        // 缺ID时按 99 号台兜底, 其余沿用卡片自身ID(如雷达 22000)
        let id = if card.id.is_empty() {
            MIXED_RADIO_ID.to_owned()
        } else {
            card.id.clone()
        };
        let client = Arc::clone(&self.client);
        let fetch_id = id.clone();
        let track = self
            .radio_queue
            .peek(&id, move |_, want| {
                let client = Arc::clone(&client);
                let id = fetch_id.clone();
                async move { pull_stream_batch(&client, &id, want).await }
            })
            .await?;
        card.cover_url = track.cover_url;
        card.title = track.title;
        Ok(())
    }
}

/// 电台/雷达下一曲批量拉取, 供给 PopQueue 的 fetch 闭包; 按 ID 分发雷达或普通电台
async fn pull_stream_batch(client: &QqClient, id: &str, want: u32) -> ProviderResult<Vec<Track>> {
    let tracks = if id == RADAR_ID {
        client.radar_next(want).await?.standardize()
    } else {
        client.radio_next(id, want).await?.standardize()
    };
    tracks.ok_or_else(|| {
        no_result(if id == RADAR_ID {
            "stream_next: radar"
        } else {
            "stream_next: radio"
        })
    })
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
