use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    auth_session,
    cache::{self, TTL_1_DAY},
    providers::{
        ProviderAdapter, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
        lyric::{LrcParser, MemchrParsers},
    },
    sidecar_log,
    types::{
        AlbumDetail, AlbumSummary, LyricPayload, PlayableState, PlaylistAddSongAck, PlaylistDetail,
        PlaylistSummary, ProviderId, ProviderLoginStatus, SongLikeAck,
        SongLikeCheckAck,
        SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    },
    utils::pop_queue::{DEFAULT_LOW_WATER, DEFAULT_RETRIES, PopQueue},
    utils::single_flight::SingleFlightCache,
};

use super::{
    client::NeteaseClient,
    lyric::{NeteaseLrcParser, NeteaseParser},
    model::{NeteaseSongUrlV1Resp, QualityCandidate, QUALITY_CANDIDATES},
};
use crate::utils::single_flight::FlightCoalescer;



const SONG_URL_TTL: std::time::Duration = std::time::Duration::from_secs(30);
const PAGE_BATCH: u32 = 200;
const PAGE_RETRIES: u32 = 2;

#[derive(Clone, Default)]
pub struct NeteaseAdapter {
    client: Arc<NeteaseClient>,
    playlist_cache: Arc<SingleFlightCache<PlaylistDetail>>,
    album_cache: Arc<SingleFlightCache<AlbumDetail>>,
    star_queue: Arc<PopQueue<Track>>,
    song_url_gate: Arc<FlightCoalescer<NeteaseSongUrlV1Resp>>,
}

impl NeteaseAdapter {
    pub fn new(client: Arc<NeteaseClient>) -> Self {
        Self {
            client,
            playlist_cache: SingleFlightCache::shared(
                ProviderId::Netease,
                PAGE_BATCH,
                PAGE_RETRIES,
            ),
            album_cache: SingleFlightCache::shared(ProviderId::Netease, PAGE_BATCH, PAGE_RETRIES),
            star_queue: Arc::new(PopQueue::new(
                ProviderId::Netease,
                DEFAULT_LOW_WATER,
                DEFAULT_RETRIES,
            )),
            song_url_gate: Arc::new(FlightCoalescer::new(SONG_URL_TTL)),
        }
    }

    async fn login_status_internal(&self) -> ProviderResult<ProviderLoginStatus> {
        let Some(cookie) = self.client.current_cookie().await else {
            return Ok(ProviderLoginStatus {
                provider: ProviderId::Netease,
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        };
        if cookie.trim().is_empty() {
            return Ok(ProviderLoginStatus {
                provider: ProviderId::Netease,
                logged_in: false,
                nickname: None,
                user_id: None,
                avatar_url: None,
                ..Default::default()
            });
        }
        let login_status = self.client.login_status().await?.standardize();
        let Some(user_id) = login_status
            .user_id
            .clone()
            .filter(|user_id| !user_id.is_empty())
        else {
            return Ok(login_status);
        };
        Ok(self
            .client
            .vip_info(&user_id)
            .await?
            .standardize(login_status))
    }

    /// v1 优先、旧接口替补的取 URL 编排; 双端点同构, 统一进同 key 单飞闸门。
    async fn resolve_song_url(
        &self,
        id: &str,
        quality: &QualityCandidate,
        logged_in: bool,
    ) -> ProviderResult<Arc<NeteaseSongUrlV1Resp>> {
        let key = format!("{logged_in}:{id}:{}", quality.level);
        self.song_url_gate
            .get(&key, || async {
                match self.client.song_url_v1(id, quality.level).await {
                    Ok(resp) => Ok(resp),
                    Err(err) => {
                        sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                            "Netease song_url_v1 回退旧接口(level={level}): {err}",
                            level = quality.level
                        )));
                        self.client.song_url(id, quality.br).await
                    }
                }
            })
            .await
    }
}

#[async_trait]
impl ProviderAdapter for NeteaseAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Netease
    }

    async fn search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Vec<Track>> {
        Ok(self
            .client
            .search_track_modeled(keyword, offset, limit)
            .await?
            .standardize())
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
            .standardize()
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
            .standardize()
            .unwrap_or_default())
    }

    async fn song_url(
        &self,
        track: &Track,
        opts: Option<SongUrlOptions>,
    ) -> ProviderResult<SongUrlResult> {
        let requested = opts
            .and_then(|value| value.quality)
            .unwrap_or_else(|| "hires".to_owned());
        let start_index = QUALITY_CANDIDATES
            .iter()
            .position(|quality| quality.level == requested)
            .unwrap_or(4);
        let logged_in = self
            .client
            .current_cookie()
            .await
            .is_some_and(|cookie| !cookie.trim().is_empty());
        let mut trial_fallback = None;
        let mut last_state = PlayableState::Unknown;
        let mut last_error = None;

        for quality in QUALITY_CANDIDATES.iter().skip(start_index) {
            // 同 key 单飞闸门: v1 优先, 同构旧接口替补
            let resp = match self.resolve_song_url(&track.source_id, quality, logged_in).await {
                Ok(resp) => resp,
                Err(err) => {
                    last_error = Some(err);
                    continue;
                }
            };
            let Some(datum) = resp.datum_for(&track.source_id) else {
                continue;
            };
            last_state = datum.playable_state(logged_in);
            if last_state != PlayableState::Playable {
                continue;
            }
            let Some(result) = datum.song_url_result() else {
                continue;
            };
            if datum.is_trial() {
                if trial_fallback.is_none() {
                    trial_fallback = Some(result);
                }
                continue;
            }
            return Ok(result);
        }

        if let Some(result) = trial_fallback {
            return Ok(result);
        }
        if let Some(err) = last_error {
            return Err(err);
        }
        Err(state_error(last_state, &track.source_id))
    }

    async fn track_qualities(&self, track: &Track) -> ProviderResult<TrackQualityAvailability> {
        self.client
            .track_qualities(&track.source_id)
            .await?
            .standardize()
            .ok_or_else(|| no_result("track_detail"))
    }


    async fn lyric(&self, track: &Track) -> ProviderResult<LyricPayload> {
        let resp = match self.client.lyric_new(&track.source_id).await {
            Ok(resp) => resp,
            Err(err) => {
                sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                    "Netease lyric_new 失败回退旧接口: {err}"
                )));
                self.client.lyric(&track.source_id).await?
            }
        };

        let lrc_text = resp.lrc.lyric.unwrap_or_default();
        let tlyric_text = resp.tlyric.lyric.unwrap_or_default();
        let yrc_text = resp.yrc.lyric.unwrap_or_default();

        let trans = (!tlyric_text.is_empty())
            .then(|| NeteaseLrcParser {}.parse(tlyric_text).ok())
            .flatten()
            .map(|t| {
                t.into_iter()
                    .map(|line| (line.time_ms, line.text))
                    .collect::<HashMap<_, _>>()
            });

        let (lines, has_translation) = {
            let base_lines = if !yrc_text.is_empty() {
                NeteaseParser
                    .parse(yrc_text)
                    .map_err(|e| invalid_response(e))?
            } else {
                NeteaseLrcParser {}
                    .parse(lrc_text)
                    .map_err(|e| invalid_response(e))?
            };

            match trans {
                Some(trans) => (
                    base_lines
                        .into_iter()
                        .map(|mut line| {
                            line.translation =
                                trans.get(&line.time_ms).cloned().filter(|v| !v.is_empty());
                            line
                        })
                        .collect(),
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
            provider: ProviderId::Netease,
            track_id: track.id.clone(),
            lines,
            has_translation,
            is_word_by_word,
        })
    }

    async fn playlist_list(&self) -> ProviderResult<Vec<PlaylistSummary>> {
        self.client.ensure_login().await?;
        let uid = self.client.login_status().await?.standardize().user_id;
        let uid = uid.unwrap_or_default();
        if uid.is_empty() {
            return Err(unavailable("missing uid".to_owned()));
        }
        self.client
            .playlist_list(&uid, 60)
            .await?
            .standardize()
            .ok_or_else(|| no_result("playlist_list"))
    }

    async fn playlist_detail(
        &self,
        id: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<PlaylistDetail> {
        if let Some(raw) = id.strip_prefix("D") {
            let (args, title) = raw.split_once('|').unzip();
            if !id.contains("categoryId") {
                return Ok(self
                    .client
                    .daily_songs()
                    .await?
                    .standardize(title.unwrap_or_default().to_string(), id.to_string()));
            }
            return Ok(self
                .client
                .daily_songs2(args.unwrap_or(raw))
                .await?
                .standardize(title.unwrap_or_default().to_string(), id.to_string()));
        }

        let client = Arc::clone(&self.client);
        let page = self
            .playlist_cache
            .get(id, offset, limit, move |id, n| {
                let client = Arc::clone(&client);
                async move { Ok(client.playlist_detail(&id, n).await?.standardize()) }
            })
            .await?;
        let mut value = page.value;
        value.has_more = Some(page.has_more);
        Ok(value)
    }

    async fn album_list(&self) -> ProviderResult<Vec<AlbumSummary>> {
        self.client.ensure_login().await?;
        Ok(self
            .client
            .album_list()
            .await?
            .standardize()
            .unwrap_or_default())
    }

    async fn album_detail(&self, id: &str, offset: u32, limit: u32) -> ProviderResult<AlbumDetail> {
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
        self.login_status_internal().await
    }

    async fn logout(&self) -> ProviderResult<()> {
        self.client.logout().await?;
        auth_session::clear_runtime_provider_cookie(&ProviderId::Netease).await;
        Ok(())
    }

    async fn like_song(&self, id: &str, liked: bool) -> ProviderResult<SongLikeAck> {
        self.client.ensure_login().await?;
        let body = self.client.like(id, liked).await?;
        Ok(SongLikeAck {
            provider: ProviderId::Netease,
            id: id.to_owned(),
            liked,
            code: Some(response_code(&body)),
        })
    }

    async fn check_song_likes(&self, ids: &[String]) -> ProviderResult<SongLikeCheckAck> {
        self.client.ensure_login().await?;
        let clean_ids = ids
            .iter()
            .filter(|id| !id.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if clean_ids.is_empty() {
            return Ok(SongLikeCheckAck {
                provider: ProviderId::Netease,
                ids: Vec::new(),
                liked: std::collections::HashMap::new(),
            });
        }

        let liked_ids = match self.client.song_like_check(&clean_ids).await {
            Ok(body) => match body
                .get("data")
                .filter(|value| !value.is_null())
                .or_else(|| body.get("ids").filter(|value| !value.is_null()))
                .unwrap_or(&body)
            {
                Value::Array(items) => items.iter().map(read_id_like).collect::<Vec<_>>(),
                Value::Object(values) => clean_ids
                    .iter()
                    .filter(|id| {
                        values
                            .get(*id)
                            .or_else(|| {
                                let numeric_id = id.parse::<u64>().ok()?.to_string();
                                values.get(&numeric_id)
                            })
                            .is_some_and(json_truthy)
                    })
                    .cloned()
                    .collect(),
                _ => Vec::new(),
            },
            Err(err) => {
                sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                    "Netease 点赞状态检查失败, 回退为空: {err}"
                )));
                Vec::new()
            }
        };

        if !liked_ids.is_empty() {
            return Ok(song_like_check_ack(&clean_ids, &liked_ids));
        }

        let status = self.login_status_internal().await?;
        let Some(uid) = status.user_id.filter(|uid| !uid.is_empty()) else {
            return Err(ProviderError {
                code: ProviderErrorCode::LoginRequired,
                provider: ProviderId::Netease,
                message: "netease like-check requires login".to_owned(),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: None,
            });
        };
        let body = self.client.likelist(&uid).await?;
        let liked_ids = body
            .get("ids")
            .and_then(Value::as_array)
            .map(|items| items.iter().map(read_id_like).collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(song_like_check_ack(&clean_ids, &liked_ids))
    }

    async fn update_song_in_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
        adding: bool,
    ) -> ProviderResult<PlaylistAddSongAck> {
        if !adding {
            return Err(ProviderError::not_implemented(
                ProviderId::Netease,
                "del_from_playlist",
            ));
        }
        //未测试
        self.client.ensure_login().await?;
        let primary = self.client.playlist_tracks(playlist_id, track_id).await?;
        let final_response = if is_successful(&primary) {
            primary
        } else {
            self.client
                .playlist_track_add(playlist_id, track_id)
                .await?
        };
        if !is_successful(&final_response) {
            return Err(ProviderError {
                code: ProviderErrorCode::Unavailable,
                provider: ProviderId::Netease,
                message: format!("netease playlist add failed for {track_id}"),
                retryable: false,
                action: None,
                raw_message: Some(final_response.to_string()),
            });
        }
        Ok(PlaylistAddSongAck {
            provider: ProviderId::Netease,
            playlist_id: playlist_id.to_owned(),
            track_id: track_id.to_owned(),
            success: true,
            code: Some(response_code(&final_response)),
        })
    }

    async fn recommendation_page(
        &self,
        refresh: bool,
    ) -> ProviderResult<crate::types::RecommendationPage> {
        let fetch = || async {
            self.client.ensure_login().await?;
            let m1 = self
                .client
                .recommendation_module1()
                .await
                .map_err(async |e| {
                    sidecar_log::log_runtime(serde_json::json!(
                        "failed to get recommendation_module1"
                    ))
                    .await;
                    e
                })
                .ok();

            Ok(self
                .client
                .recommendation_page()
                .await?
                .standardize(m1)
                .and_then(|page| serde_json::to_string(&page).ok()))
        };
        let raw = if refresh {
            let raw = fetch().await?;
            if let Some(value) = raw.as_ref() {
                cache::insert(
                    ProviderId::Netease,
                    "recommendation_page",
                    TTL_1_DAY,
                    value.clone(),
                )
                .await;
            }
            raw
        } else {
            cache::get_or_refresh(ProviderId::Netease, "recommendation_page", TTL_1_DAY, fetch)
                .await?
        };
        raw.and_then(|raw| serde_json::from_str(&raw).ok())
            .ok_or_else(|| no_result("recommendation_page"))
    }

    async fn stream_next(&self, id: &str) -> ProviderResult<Track> {
        match id.chars().next() {
            Some('P') => self
                .client
                .personal_fm()
                .await?
                .standardize()
                .ok_or_else(|| unavailable("personal_fm".to_owned())),

            Some('S') => {
                let (pid, tid) = id
                    .strip_prefix('S')
                    .and_then(|id| id.split_once('|'))
                    .ok_or_else(|| unavailable("star_mode: get id".to_owned()))?;
                let client = Arc::clone(&self.client);

                self.star_queue
                    .pop(pid, move |seed, want| {
                        // 种子: 低水位补货用刚弹出的那条, 冷启动用请求串里的 tid 兜底;
                        // 上游持续失败导致队列清空时, 冷启动路径始终有 tid 可用
                        let seed_id = seed.map(|t| t.id.clone()).unwrap_or_else(|| tid.to_owned());
                        let client = Arc::clone(&client);
                        let pid = pid.to_owned();
                        async move {
                            client
                                .star_mode(&pid, &seed_id, want)
                                .await?
                                .standardize()
                                .ok_or_else(|| unavailable("star_mode".to_owned()))
                        }
                    })
                    .await
            }

            _ => Err(unavailable("stream_next: invalid id".to_owned())),
        }
    }
}

fn response_code(body: &Value) -> i64 {
    body.get("code")
        .and_then(Value::as_f64)
        .filter(|code| code.is_finite())
        .map(|code| code.floor() as i64)
        .unwrap_or(200)
}

fn is_successful(body: &Value) -> bool {
    response_code(body) == 200 && !body.get("error").is_some_and(json_truthy)
}

fn json_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn read_id_like(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => String::new(),
    }
}

fn song_like_check_ack(ids: &[String], liked_ids: &[String]) -> SongLikeCheckAck {
    let liked_set = liked_ids
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    SongLikeCheckAck {
        provider: ProviderId::Netease,
        ids: ids.to_vec(),
        liked: ids
            .iter()
            .map(|id| (id.clone(), liked_set.contains(id)))
            .collect(),
    }
}

fn invalid_response(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::InvalidResponse,
        provider: ProviderId::Netease,
        message,
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn state_error(state: PlayableState, id: &str) -> ProviderError {
    let code = match state {
        PlayableState::VipRequired => ProviderErrorCode::VipRequired,
        PlayableState::CopyrightUnavailable => ProviderErrorCode::CopyrightUnavailable,
        _ => ProviderErrorCode::Unavailable,
    };
    ProviderError {
        code,
        provider: ProviderId::Netease,
        message: format!("netease song-url {id} state {state}"),
        // 登录相关的重试/action 提示随 songurl 建模迁移到错误码路径
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn no_result(action: &str) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::NoResult,
        provider: ProviderId::Netease,
        message: format!("{} no result", action),
        retryable: false,
        action: Some(action.to_string()),
        raw_message: None,
    }
}

fn unavailable(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: ProviderId::Netease,
        message,
        retryable: false,
        action: None,
        raw_message: None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use std::sync::Arc;

    use crate::types::{AlbumDetail, ProviderId, Track};
    use crate::utils::single_flight::SingleFlightCache;

    use super::response_code;

    #[test]
    fn response_code_defaults_only_when_the_code_field_is_missing_or_non_numeric() {
        assert_eq!(response_code(&json!({ "code": 201 })), 201);
        assert_eq!(response_code(&json!({ "code": "201" })), 200);
    }

    /// 缓存路径下切片保留元数据、正确截取曲目并给出 has_more
    #[tokio::test]
    async fn cache_slices_pages_and_preserves_metadata() {
        let detail = AlbumDetail {
            provider: ProviderId::Netease,
            id: "1".to_owned(),
            name: "专辑名".to_owned(),
            artists: vec!["歌手".to_owned()],
            cover_url: "https://example.com/c.jpg".to_owned(),
            track_count: Some(3),
            track_ids: vec!["1".to_owned(), "2".to_owned(), "3".to_owned()],
            collected: None,
            tracks: vec![
                Track {
                    source_id: "1".to_owned(),
                    title: "歌1".to_owned(),
                    ..Default::default()
                },
                Track {
                    source_id: "2".to_owned(),
                    title: "歌2".to_owned(),
                    ..Default::default()
                },
                Track {
                    source_id: "3".to_owned(),
                    title: "歌3".to_owned(),
                    ..Default::default()
                },
            ],
            has_more: None,
        };
        let data = Arc::new(detail);
        let cache = SingleFlightCache::<AlbumDetail>::new(ProviderId::Netease, 200, 0);
        let fetch_data = Arc::clone(&data);

        let page = cache
            .get("1", 1, 1, move |_, _| {
                let data = Arc::clone(&fetch_data);
                async move { Ok((*data).clone()) }
            })
            .await
            .expect("page");
        assert_eq!(page.value.name, "专辑名");
        assert_eq!(page.value.artists, vec!["歌手"]);
        assert_eq!(page.value.cover_url, "https://example.com/c.jpg");
        assert_eq!(page.value.track_count, Some(3));
        assert_eq!(page.value.track_ids, vec!["2"]);
        assert_eq!(page.value.tracks.len(), 1);
        assert_eq!(page.value.tracks[0].title, "歌2");
        assert!(page.has_more);

        // 区间越过缓存末尾: 触发扩容重取后仍得到空页
        let beyond = cache
            .get("1", 5, 2, move |_, _| {
                let data = Arc::clone(&data);
                async move { Ok((*data).clone()) }
            })
            .await
            .expect("page");
        assert!(beyond.value.tracks.is_empty());
        assert!(beyond.value.track_ids.is_empty());
        assert_eq!(beyond.value.name, "专辑名");
        assert!(!beyond.has_more);
    }
}
