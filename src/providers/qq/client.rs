use std::sync::Arc;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use reqwest::{
    Client,
    header::{COOKIE, HeaderMap, HeaderName, HeaderValue, ORIGIN, REFERER, USER_AGENT},
};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::utils::{
    cookie::Cookie,
    cryptors::qq::{x4, x5, x7, x9, xj},
};
use crate::{
    auth_session,
    providers::{
        ProviderId, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
        qq::model::{
            QqAlbumDetailResp, QqAlbumListResp, QqCdnDispatch, QqCdnTestResp, QqLoginStatusResp,
            QqLyricResp, QqMultiSearchResp, QqPlaylistDetailResp, QqPlaylistList1Resp,
            QqPlaylistList2Resp, QqPlaylistSongWriteResp, QqRadarResp, QqRadioDetailResp,
            QqRecommendationResp, QqSearchResp, QqSongUrlResp, QqTrackDetailResp, QqTrackInfo,
            QqVipIconResp,
        },
    },
    qr_login::common::normalize_login_cookie,
    sidecar_log,
};

const UA: &str = "Mozilla/5.0";

#[derive(Clone, Default)]
pub struct QqClient {
    http: Client,
    uin: Arc<RwLock<Option<String>>>,
    euin: Arc<RwLock<Option<String>>>,
    cdn_cache: Arc<RwLock<Option<QqCdnCache>>>,
}

struct QqCdnCache {
    cdn: String,
    expires_at: Instant,
}

impl QqClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            uin: Arc::new(RwLock::new(None)),
            euin: Arc::new(RwLock::new(None)),
            cdn_cache: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn current_cookie(&self) -> Option<String> {
        auth_session::get_provider_cookie(&ProviderId::Qq).await
    }

    pub(super) async fn ensure_login(&self) -> ProviderResult<()> {
        if self
            .current_cookie()
            .await
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            return Err(ProviderError {
                code: ProviderErrorCode::LoginRequired,
                provider: ProviderId::Qq,
                message: "qq login required".to_owned(),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: None,
            });
        }
        Ok(())
    }

    pub async fn uin(&self) -> Option<String> {
        if let Some(uin) = self.uin.read().await.clone() {
            return Some(uin);
        }

        let cookie = self.current_cookie().await?;
        let uin = uin_from_cookie(&cookie)?;

        *self.uin.write().await = Some(uin.clone());
        Some(uin)
    }

    pub async fn guid(&self) -> Option<String> {
        let c = Cookie::new(&self.current_cookie().await?);
        c.find("qqmusic_guid")
    }

    pub async fn euin(&self) -> Option<String> {
        if let Some(euin) = self.euin.read().await.clone() {
            return Some(euin);
        }

        let cookie = self.current_cookie().await?;
        if let Some(euin) = euin_from_cookie(&cookie) {
            self.set_euin(euin.clone()).await;
            return Some(euin);
        }

        let uin = self.uin().await?;
        if let Err(err) = self.login_status_with_cookie(&uin, &cookie).await {
            sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                "QQ 刷新 euin 登录状态失败: {err}"
            )));
        }
        if let Some(euin) = self.euin.read().await.clone() {
            return Some(euin);
        }
        None
    }

    async fn set_euin(&self, euin: String) {
        let euin = euin.trim().to_owned();
        if !euin.is_empty() {
            *self.euin.write().await = Some(euin);
        }
    }

    #[allow(dead_code)]
    pub fn get_sign(&self, payload: &Value) -> ProviderResult<String> {
        let payload = serde_json::to_string(payload).map_err(|err| unavailable_error(err))?;
        Ok(x9(&payload))
    }

    pub(super) async fn search(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QqSearchResp> {
        let url = "https://shc.y.qq.com/soso/fcgi-bin/search_for_qq_cp";
        let page = (offset / limit.max(1) + 1).to_string();
        let query = [
            ("format", "json".to_owned()),
            ("n", limit.to_string()),
            ("p", page),
            ("w", keyword.to_owned()),
            ("cr", "1".to_owned()),
            ("g_tk", "5381".to_owned()),
            ("t", "0".to_owned()),
        ];
        let response = self
            .http
            .get(url)
            .query(&query)
            .headers(build_headers(
                Some("https://y.qq.com"),
                self.current_cookie().await.as_deref(),
                false,
            )?)
            .send()
            .await
            .context("send qq search request")
            .map_err(unavailable_error)?;
        let body = response
            .bytes()
            .await
            .context("read qq search response")
            .map_err(unavailable_error)?;

        serde_json::from_slice(&body).map_err(|err| ProviderError {
            code: ProviderErrorCode::InvalidResponse,
            provider: ProviderId::Qq,
            message: format!("decode qq search response: {err}"),
            retryable: false,
            action: Some("search".to_owned()),
            raw_message: Some(String::from_utf8_lossy(&body).into_owned()),
        })
    }

    pub(super) async fn search_album(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QqMultiSearchResp> {
        let page_num = (offset / limit.max(1) + 1).to_string();
        self.post_json_with_sign(
            json!({
                "result": {
                    "method": "DoSearchForQQMusicDesktop",
                    "module": "music.search.SearchCgiService",
                    "param": {
                        "grp": 0,
                        "num_per_page": limit,
                        "page_num": page_num,
                        "query": keyword,
                        "search_type": 2,
                        "searchid": ""
                    }
                }
            }),
            Some("https://y.qq.com/"),
            self.current_cookie().await.as_deref(),
            "search_album",
            true,
        )
        .await
    }

    pub(super) async fn search_playlist(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QqMultiSearchResp> {
        let page_num = (offset / limit.max(1) + 1).to_string();
        self.post_json_with_sign(
            json!({
                "result": {
                    "method": "DoSearchForQQMusicDesktop",
                    "module": "music.search.SearchCgiService",
                    "param": {
                        "grp": 0,
                        "num_per_page": limit,
                        "page_num": page_num,
                        "query": keyword,
                        "search_type": 3,
                        "searchid": ""
                    }
                }
            }),
            Some("https://y.qq.com/"),
            self.current_cookie().await.as_deref(),
            "search_playlist",
            true,
        )
        .await
    }

    pub(super) async fn multi_search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QqMultiSearchResp> {
        let page_num = (offset / limit.max(1) + 1).to_string();
        self.post_json_with_sign(
            json!({
                "result": {
                    "method": "DoSearchForQQMusicDesktop",
                    "module": "music.search.SearchCgiService",
                    "param": {
                        "grp": 0,
                        "num_per_page": limit,
                        "page_num": page_num,
                        "query": keyword,
                        "search_type": 0,
                        "searchid": ""
                    }
                }
            }),
            Some("https://y.qq.com/"),
            self.current_cookie().await.as_deref(),
            "multi_search_track",
            true,
        )
        .await
    }

    pub(super) async fn song_detail(&self, song_mid: &str) -> ProviderResult<QqTrackDetailResp> {
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "method": "get_song_detail_yqq",
                    "module": "music.pf_song_detail_svr",
                    "param": { "song_mid": song_mid }
                }
            }),
            None,
            self.current_cookie().await.as_deref(),
            "song_detail",
            true,
        )
        .await
    }

    pub(super) async fn song_url(
        &self,
        song_mid: &str,
        filenames: String,
        encrypted: bool,
    ) -> ProviderResult<QqSongUrlResp> {
        let cookie = self.current_cookie().await;
        let c = Cookie::new(cookie.as_deref().unwrap_or_default());
        let uin = self.uin().await.unwrap_or_else(|| "0".to_owned());
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "module": if encrypted { "music.vkey.GetEVkey" } else { "music.vkey.GetVkey" },
                    "method": if encrypted { "CgiGetEVkey" } else { "UrlGetVkey" },
                    "param": {
                        "checklimit": 0,
                        "ctx": 1,
                        "downloadfrom": if encrypted { 0 } else { 1 },
                        "filename": [filenames],
                        "guid": c.find_or_else("qqmusic_guid", x5),
                        "musicfile": [filenames],
                        "nettype": "",
                        "referer": "y.qq.com",
                        "scene": 0,
                        "songmid": [song_mid],
                        "songtype": [1],
                        "uin": uin,
                    },
                }
            }),
            None,
            cookie.as_deref(),
            "song_url",
            true,
        )
        .await
    }

    pub(super) async fn cdn(&self) -> ProviderResult<String> {
        if let Some(cache) = self.cdn_cache.read().await.as_ref()
            && cache.expires_at > Instant::now()
        {
            return Ok(cache.cdn.clone());
        }

        let mut cache = self.cdn_cache.write().await;
        if let Some(cache) = cache.as_ref()
            && cache.expires_at > Instant::now()
        {
            return Ok(cache.cdn.clone());
        }

        let dispatch = self.cdn_dispatch().await?;
        let cdn = self
            .fastest_cdn(&dispatch)
            .await
            .ok_or_else(|| ProviderError {
                code: ProviderErrorCode::NoUrl,
                provider: ProviderId::Qq,
                message: "qq cdn speed test failed".to_owned(),
                retryable: true,
                action: None,
                raw_message: None,
            })?;
        *cache = Some(QqCdnCache {
            cdn: cdn.clone(),
            expires_at: Instant::now() + Duration::from_secs(60),
        });
        Ok(cdn)
    }

    async fn cdn_dispatch(&self) -> ProviderResult<QqCdnDispatch> {
        let cookie = self.current_cookie().await;
        let uin = self.uin().await.unwrap_or_else(|| "0".to_owned());
        let response: QqCdnTestResp = self
            .post_json_with_sign(
                json!({
                    "modulecdn": {
                        "module": "music.audioCdnDispatch.cdnDispatch",
                        "method": "GetCdnDispatch",
                        "param": {
                            "ctx": 1,
                            "guid": x5(),
                            "referer": "y.qq.com",
                            "scene": 0,
                            "uin": uin,
                        }
                    }
                }),
                None,
                cookie.as_deref(),
                "cdn_dispatch",
                true,
            )
            .await?;
        response.standardize().ok_or_else(|| ProviderError {
            code: ProviderErrorCode::NoUrl,
            provider: ProviderId::Qq,
            message: "qq cdn dispatch returned no usable SIP".to_owned(),
            retryable: true,
            action: None,
            raw_message: None,
        })
    }

    async fn fastest_cdn(&self, dispatch: &QqCdnDispatch) -> Option<String> {
        let mut fastest = None;
        for sip in &dispatch.sips {
            let test_url = format!("{}{}", sip, dispatch.test_file);
            let started = Instant::now();
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                let response = self.http.get(test_url).send().await.ok()?;
                response.status().is_success().then_some(())?;
                response.bytes().await.ok().map(|_| ())
            })
            .await;
            if matches!(result, Ok(Some(()))) {
                let elapsed = started.elapsed();
                if fastest.as_ref().is_none_or(|(best, _)| elapsed < *best) {
                    fastest = Some((elapsed, sip.clone()));
                }
            }
        }
        fastest.map(|(_, sip)| sip)
    }

    pub(super) async fn lyric(&self, song_mid: &str) -> ProviderResult<QqLyricResp> {
        self.post_json_with_sign(
            json!({"req_0": {
                "method": "GetPlayLyricInfo",
                "module": "music.musichallSong.PlayLyricInfo",
                "param": {
                "crypt": 1,
                "qrc": 1,
                "songMID": song_mid,
                "trans": 1,
                "type": 0
                }
            }
            }),
            None,
            self.current_cookie().await.as_deref(),
            "lyric",
            true,
        )
        .await
    }

    pub(super) async fn login_status_with_cookie(
        &self,
        user_id: &str,
        cookie: &str,
    ) -> ProviderResult<QqLoginStatusResp> {
        let body: QqLoginStatusResp = self
            .get_model(
                &format!("https://c.y.qq.com/rsc/fcgi-bin/fcg_get_profile_homepage.fcg?cid=205360838&ct=20&cv=2230&userid={}&reqfrom=1&reqtype=0", user_id),
                None,
                Some(cookie),
                "login_status",
            )
            .await?;
        self.set_euin(body.encrypted_uin()).await;
        Ok(body)
    }

    pub(super) async fn refresh_login_cookie(
        &self,
        cookie: &str,
    ) -> ProviderResult<Option<String>> {
        let c = Cookie::new(cookie);
        let Some((request_key, body, is_wechat)) = login_refresh_request(&c.map) else {
            return Ok(None);
        };
        let response: Value = self
            .post_json_with_sign(body, None, Some(cookie), "login_refresh", true)
            .await?;
        let Some(data) = response
            .get(request_key)
            .filter(|value| {
                value
                    .get("code")
                    .and_then(Value::as_i64)
                    .unwrap_or_default()
                    == 0
            })
            .and_then(|value| value.get("data"))
        else {
            return Ok(None);
        };
        let guid = c.find_or_else("qqmusic_guid", x5);
        let refreshed = normalize_login_cookie(data, &guid, is_wechat, "QQ_LOGIN_REFRESH_EMPTY")
            .map_err(internal_error)?;
        Ok(Some(merge_cookie(c, &refreshed)))
    }

    pub(super) async fn vip_info_with_cookie(
        &self,
        user_id: &str,
        cookie: &str,
    ) -> ProviderResult<QqVipIconResp> {
        self.post_json_with_sign(
            json!({
                "getVipIcon": {
                    "module": "music.lvz.VipIconUiShowSvr",
                    "method": "GetVipIconUiV2",
                    "param": { "Encuin": user_id, "PID": 8 }
                }
            }),
            Some("https://y.qq.com/m/myservice/index.html"),
            Some(cookie),
            "vip_info",
            true,
        )
        .await
    }

    pub(super) async fn user_songlists(&self, euin: &str) -> ProviderResult<QqPlaylistList1Resp> {
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "method": "GetPlaylistByUin",
                    "module": "music.musicasset.PlaylistBaseRead",
                    "param": {
                        "euin": euin
                    }
                }
            }),
            None,
            self.current_cookie().await.as_deref(),
            "playlist_list",
            true,
        )
        .await
    }

    pub(super) async fn user_collect_songlists(
        &self,
        uin: &str,
    ) -> ProviderResult<QqPlaylistList2Resp> {
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "method": "GetPlaylistFavInfo",
                    "module": "music.musicasset.PlaylistFavRead",
                    "param": {
                        "uin": uin
                    }
                }
            }),
            Some("https://y.qq.com/"),
            self.current_cookie().await.as_deref(),
            "playlist_detail",
            true,
        )
        .await
    }

    pub(super) async fn playlist_detail(
        &self,
        playlist_id: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QqPlaylistDetailResp> {
        let disstid = playlist_id.trim().parse::<u64>().map_err(internal_error)?;
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "module": "music.srfDissInfo.DissInfoForPc",
                    "method": "uniform_get_Dissinfo",
                    "param": {
                        "disstid": disstid,
                        "userinfo": 1,
                        "tag": 1,
                        "orderlist": 1,
                        "song_begin": offset,
                        "song_num": limit.clamp(1, 500),
                        "onlysonglist": 0,
                        "enc_host_uin": ""
                    }
                }
            }),
            Some("https://y.qq.com/"),
            self.current_cookie().await.as_deref(),
            "playlist_detail",
            true,
        )
        .await
    }

    pub(super) async fn radio_next(
        &self,
        playlist_id: &str,
        count: u32,
    ) -> ProviderResult<QqRadioDetailResp> {
        let disstid = playlist_id.trim().parse::<u64>().map_err(internal_error)?;
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "method": "get_radio_track",
                    "module": "music.radioProxy.MbTrackRadioSvr",
                    "param": {
                        "id": disstid,
                        "num": count.max(1)
                    }
                }
            }),
            Some("https://y.qq.com/"),
            self.current_cookie().await.as_deref(),
            "radio_detail",
            true,
        )
        .await
    }

    pub(super) async fn radar_next(&self, count: u32) -> ProviderResult<QqRadarResp> {
        let now = SystemTime::now();
        let since_epoch = now
            .duration_since(UNIX_EPOCH)
            .expect("系统时间早于 UNIX 纪元");
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "method": "GetRadarSong",
                    "module": "music.recommend.TrackRelationServer",
                    "param": {
                        "LastToastTime": &since_epoch.as_secs(),
                        "NeedNum": count.max(1),
                        "Page": 1,
                        "ReqType": 0
                    }
                }
            }),
            Some("https://y.qq.com/"),
            self.current_cookie().await.as_deref(),
            "radio_detail",
            true,
        )
        .await
    }
    pub(super) async fn album_list(&self) -> ProviderResult<QqAlbumListResp> {
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "method": "CgiGetAlbumFavInfo",
                    "module": "music.musicasset.AlbumFavRead",
                    "param": {
                        "euin": self.euin().await.unwrap_or_default(),
                        "offset": 0,
                        "size": 48
                    }
                }
            }),
            Some("https://y.qq.com/"),
            self.current_cookie().await.as_deref(),
            "album_list",
            true,
        )
        .await
    }

    pub(super) async fn album_detail(
        &self,
        mid: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QqAlbumDetailResp> {
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "module": "music.musichallAlbum.AlbumSongList",
                    "method": "GetAlbumSongList",
                    "param": {
                        "albumMid": mid,
                        "begin": offset,
                        "num": limit,
                        "order": 2
                    }
                },
                "req_1": {
                    "module": "music.musichallAlbum.AlbumInfoServer",
                    "method": "GetAlbumDetail",
                    "param": { "albumMid": mid }
                }
            }),
            Some("https://y.qq.com/"),
            self.current_cookie().await.as_deref(),
            "album_detail",
            true,
        )
        .await
    }

    pub(super) async fn update_song_in_playlist(
        &self,
        dir_id: u64,
        track_id: &str,
        adding: bool,
    ) -> ProviderResult<QqPlaylistSongWriteResp> {
        let method = if adding { "AddSonglist" } else { "DelSonglist" };
        self.write_playlist_song(method, dir_id, track_id).await
    }

    async fn write_playlist_song(
        &self,
        method: &str,
        dir_id: u64,
        track_id: &str,
    ) -> ProviderResult<QqPlaylistSongWriteResp> {
        self.post_json_with_sign(
            playlist_song_write_body(method, dir_id, track_id),
            None,
            self.current_cookie().await.as_deref(),
            "playlist_song_write",
            true,
        )
        .await
    }

    pub async fn logout(&self) -> ProviderResult<IgnoredAny> {
        self.post_json_with_sign(
            json!({
                "music.login.LoginServer.Logout": {
                    "method": "Logout",
                    "module": "music.login.LoginServer",
                    "param": {}
                }
            }),
            None,
            self.current_cookie().await.as_deref(),
            "logout",
            true,
        )
        .await
    }

    pub(super) async fn recommend_page(&self) -> ProviderResult<QqRecommendationResp> {
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "module": "music.recommend.RecommendFeed",
                    "method": "get_recommend_feed",
                    "param": {
                        "direction": 0,
                        "page": 1,
                        "v_cache": [],
                        "v_uniq": [],
                        "s_num": 0
                    }
                }
            }),
            Some("https://i2.y.qq.com/n3/wk_v20/entry/index/recommend?nosaveurl=1"),
            self.current_cookie().await.as_deref(),
            "recommend_page",
            false,
        )
        .await
    }

    pub(super) async fn get_track_info_by_ids(&self, ids: Vec<u32>) -> ProviderResult<QqTrackInfo> {
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "module": "music.trackInfo.UniformRuleCtrl",
                    "method": "CgiGetTrackInfo",
                    "param": {
                        "ids": ids,
                        "types": vec![200; ids.len()]
                    }
                },
            }),
            None,
            self.current_cookie().await.as_deref(),
            "get_mids_by_ids",
            false,
        )
        .await
    }

    pub(super) async fn get_track_info_by_mids(
        &self,
        ids: Vec<String>,
    ) -> ProviderResult<QqTrackInfo> {
        self.post_json_with_sign(
            json!({
                "req_0": {
                    "module": "music.trackInfo.UniformRuleCtrl",
                    "method": "CgiGetTrackInfo",
                    "param": {
                        "mids": ids,
                        "types": vec![200; ids.len()]
                    }
                },
            }),
            None,
            self.current_cookie().await.as_deref(),
            "get_mids_by_ids",
            false,
        )
        .await
    }

    async fn post_json_with_sign<T: DeserializeOwned>(
        &self,
        body: Value,
        referer: Option<&str>,
        cookie: Option<&str>,
        action: &str,
        s: bool,
    ) -> ProviderResult<T> {
        let now = SystemTime::now();
        let since_epoch = now
            .duration_since(UNIX_EPOCH)
            .expect("系统时间早于 UNIX 纪元");
        //构建鉴权部分
        let mut req = body;
        let c = Cookie::new(cookie.unwrap_or_default());

        let cookie_keys = vec!["psrf_qqaccess_token", "psrf_qqopenid", "psrf_qqunionid"];

        let mut comm_obj = serde_json::Map::new();

        comm_obj.insert("_channelid".to_string(), json!("20"));
        comm_obj.insert("_os_version".to_string(), json!("6.2.9200-2"));
        comm_obj.insert(
            "authst".to_string(),
            json!(c.find_or_default::<String>("qm_keyst")),
        );

        comm_obj.insert("format".to_string(), json!("json"));
        comm_obj.insert("platform".to_string(), json!("wk_v17"));
        comm_obj.insert("inCharset".to_string(), json!("utf-8"));
        comm_obj.insert("outCharset".to_string(), json!("utf-8"));
        comm_obj.insert("notice".to_string(), json!(0));
        comm_obj.insert("needNewCode".to_string(), json!(1));
        comm_obj.insert("ct".to_string(), json!("20"));
        comm_obj.insert("cv".to_string(), json!("2230"));
        comm_obj.insert(
            "guid".to_string(),
            json!(c.find_or_else("qqmusic_guid", x5)),
        );
        comm_obj.insert("patch".to_string(), json!("118"));
        comm_obj.insert(
            "psrf_access_token_expiresAt".to_string(),
            json!(c.find_or_default::<u128>("psrf_access_token_expiresAt")),
        );

        for key in cookie_keys {
            comm_obj.insert(key.to_string(), json!(c.find_or_default::<String>(key)));
        }

        comm_obj.insert("tmeAppID".to_string(), json!("qqmusic"));
        comm_obj.insert(
            "tmeLoginType".to_string(),
            json!(c.find_or("tmeLoginType", 2)),
        );
        comm_obj.insert(
            "uin".to_string(),
            json!(self.uin().await.unwrap_or_default()),
        );
        comm_obj.insert("wid".to_string(), json!("4810302018970526720"));
        let g_tk = x7(&c.find_or_default::<String>("musickey")).to_string();
        comm_obj.insert("g_tk_new_20200303".to_string(), json!(&g_tk));
        comm_obj.insert("g_tk".to_string(), json!(&g_tk));
        drop(c);
        if let Some(obj) = req.as_object_mut() {
            obj.insert("comm".to_string(), comm_obj.into());
        }
        let t = match s {
            true => since_epoch.as_secs(),
            _ => since_epoch.as_secs() * 1000 + since_epoch.subsec_millis() as u64,
        }
        .to_string();
        let sign = self.get_sign(&req)?;
        let mut h = build_headers(referer, cookie, false)?;
        let q = [xj(0x7063_6163_6865_7469), xj(0x6d65)].concat();
        let query: Vec<(&str, &str)> = if s {
            let (a, b) = x4(&req.to_string(), since_epoch.as_secs());
            h.insert(
                HeaderName::from_bytes(xj(0x5369_676e).as_bytes()).map_err(internal_error)?,
                header_value(&a)?,
            );
            h.insert(
                HeaderName::from_bytes(xj(0x4d61_736b).as_bytes()).map_err(internal_error)?,
                header_value(&b)?,
            );

            vec![(&q, &t)]
        } else {
            vec![("sign", &sign), ("_", &t)]
        };
        let response = self
            .http
            .post("https://u.y.qq.com/cgi-bin/musics.fcg")
            .query(&query)
            .headers(h)
            .json(&req)
            .send()
            .await
            .context("send qq upstream post request")
            .map_err(unavailable_error)?;
        let raw = response
            .bytes()
            .await
            .context("read qq upstream response")
            .map_err(unavailable_error)?;
        serde_json::from_slice(&raw).map_err(|err| ProviderError {
            code: ProviderErrorCode::InvalidResponse,
            provider: ProviderId::Qq,
            message: format!("decode qq {action} response: {err}"),
            retryable: false,
            action: Some(action.to_owned()),
            raw_message: Some(String::from_utf8_lossy(&raw).into_owned()),
        })
    }

    async fn get_model<T: DeserializeOwned>(
        &self,
        url: &str,
        referer: Option<&str>,
        cookie: Option<&str>,
        action: &str,
    ) -> ProviderResult<T> {
        let headers = build_headers(referer, cookie, true)?;
        let response = self
            .http
            .get(url)
            .headers(headers)
            .send()
            .await
            .context("send qq upstream post request")
            .map_err(unavailable_error)?;
        let status = response.status();
        let raw = response
            .bytes()
            .await
            .context("read qq upstream response")
            .map_err(unavailable_error)?;
        if !status.is_success() {
            return Err(ProviderError {
                code: ProviderErrorCode::Unavailable,
                provider: ProviderId::Qq,
                message: format!("qq {action} upstream returned HTTP {}", status.as_u16()),
                retryable: status.is_server_error(),
                action: Some(action.to_owned()),
                raw_message: Some(String::from_utf8_lossy(&raw).into_owned()),
            });
        }
        serde_json::from_slice(&raw).map_err(|err| ProviderError {
            code: ProviderErrorCode::InvalidResponse,
            provider: ProviderId::Qq,
            message: format!("decode qq {action} response: {err}"),
            retryable: false,
            action: Some(action.to_owned()),
            raw_message: Some(String::from_utf8_lossy(&raw).into_owned()),
        })
    }
}

fn build_headers(
    referer: Option<&str>,
    cookie: Option<&str>,
    with_origin: bool,
) -> ProviderResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(UA));
    if let Some(referer) = referer {
        headers.insert(REFERER, header_value(referer)?);
        if with_origin {
            let origin = reqwest::Url::parse(referer)
                .ok()
                .and_then(|url| {
                    Some(format!(
                        "{}://{}",
                        url.scheme(),
                        url.host_str().unwrap_or_default()
                    ))
                })
                .unwrap_or_else(|| "https://y.qq.com".to_owned());
            headers.insert(ORIGIN, header_value(&origin)?);
        }
    }
    if let Some(cookie) = cookie.filter(|value| !value.trim().is_empty()) {
        headers.insert(COOKIE, header_value(cookie)?);
    }
    Ok(headers)
}

fn playlist_song_write_body(method: &str, dir_id: u64, track_id: &str) -> Value {
    json!({"req_0": {
        "method": method,
        "module": "music.musicasset.PlaylistDetailWrite",
        "param": {
            "bFmtUtf8": true,
            "dirId": dir_id,
            "v_songInfo": [
                {
                    "songMid": track_id,
                    "songType": 0
                }
            ]
        }
    }
    })
}

fn uin_from_cookie(cookie: &str) -> Option<String> {
    let cookie = Cookie::new(cookie);
    let raw = cookie.first::<String>(&["wxuin", "qqmusic_uin", "uin"])?;

    let digits: String = raw.chars().filter(|ch| ch.is_ascii_digit()).collect();

    (!digits.is_empty()).then_some(digits)
}

fn euin_from_cookie(cookie: &str) -> Option<String> {
    let cookie = Cookie::new(cookie);
    cookie.first::<String>(&["encrypt_uin", "euin"])
}

fn qq_playback_key_from_cookie_map(cookie: &std::collections::HashMap<String, String>) -> String {
    [
        "qm_keyst",
        "qqmusic_key",
        "music_key",
        "p_skey",
        "skey",
        "psrf_qqaccess_token",
        "psrf_qqrefresh_token",
        "wxrefresh_token",
        "wxskey",
    ]
    .into_iter()
    .find_map(|key| cookie.get(key).cloned())
    .unwrap_or_default()
}

fn cookie_key(cookie: &std::collections::HashMap<String, String>, key: &str) -> Option<String> {
    cookie.get(key).cloned()
}

fn login_refresh_request(
    cookie: &std::collections::HashMap<String, String>,
) -> Option<(&'static str, Value, bool)> {
    let device_name = std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .map(|name| format!("{name}-MRT"))
        .unwrap_or_else(|| "MineRadio-MRT".to_owned());
    login_refresh_request_for_device(cookie, device_name)
}

fn login_refresh_request_for_device(
    cookie: &std::collections::HashMap<String, String>,
    device_name: String,
) -> Option<(&'static str, Value, bool)> {
    let login_type = cookie_key(cookie, "tmeLoginType")?.parse::<u8>().ok()?;
    let openid = cookie_key(cookie, "psrf_qqopenid").unwrap_or_default();
    let musickey = cookie_key(cookie, "qm_keyst").or_else(|| cookie_key(cookie, "qqmusic_key"))?;
    let expired_in = cookie_number_or_zero(cookie, "expired_in");
    let musicid = cookie_key(cookie, "musicid").or_else(|| cookie_key(cookie, "qqmusic_uin"))?;
    let refresh_key = required_cookie_key(cookie, "refresh_key")?;
    let access_token = cookie_key(cookie, "psrf_qqaccess_token").unwrap_or_default();
    let refresh_token = cookie_key(cookie, "psrf_qqrefresh_token")
        .or_else(|| cookie_key(cookie, "wxrefresh_token"))
        .unwrap_or_default();
    let (request_key, appid_key, appid, is_wechat) = match login_type {
        1 => (
            "WXLoginByToken",
            "strAppid",
            json!("wx48db31d50e334801"),
            true,
        ),
        2 => (
            "music.login.LoginServer.Login",
            "appid",
            json!(100497308),
            false,
        ),
        _ => return None,
    };
    let mut param = serde_json::Map::from_iter([
        ("openid".to_owned(), json!(openid)),
        ("musickey".to_owned(), json!(musickey)),
        ("expired_in".to_owned(), expired_in),
        ("musicid".to_owned(), cookie_number_or_string(musicid)),
        ("onlyNeedAccessToken".to_owned(), json!(0)),
        (appid_key.to_owned(), appid),
        ("deviceName".to_owned(), json!(device_name)),
        ("deviceType".to_owned(), json!("Widnows")),
        ("refresh_key".to_owned(), json!(refresh_key)),
        ("access_token".to_owned(), json!(access_token)),
        ("refresh_token".to_owned(), json!(refresh_token)),
    ]);
    param.insert("forceRefreshToken".to_owned(), json!(0));
    let mut body = serde_json::Map::new();
    body.insert(
        request_key.to_owned(),
        json!({
            "module": "music.login.LoginServer",
            "method": "Login",
            "param": param,
        }),
    );
    Some((request_key, Value::Object(body), is_wechat))
}

fn required_cookie_key(
    cookie: &std::collections::HashMap<String, String>,
    key: &str,
) -> Option<String> {
    cookie_key(cookie, key).filter(|value| !value.trim().is_empty())
}

fn cookie_number_or_zero(cookie: &std::collections::HashMap<String, String>, key: &str) -> Value {
    cookie_key(cookie, key)
        .and_then(|value| value.parse::<u64>().ok())
        .map(Value::from)
        .unwrap_or_else(|| Value::from(0))
}

fn cookie_number_or_string(value: String) -> Value {
    value
        .parse::<u64>()
        .map(Value::from)
        .unwrap_or(Value::String(value))
}

fn merge_cookie(existing: Cookie, replacement: &str) -> String {
    let mut merged = existing.map;
    merged.extend(Cookie::new(replacement).map);
    let mut entries: Vec<_> = merged.into_iter().collect();
    entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    entries
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn header_value(value: &str) -> ProviderResult<HeaderValue> {
    HeaderValue::from_str(value).map_err(internal_error)
}

fn internal_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Internal,
        provider: ProviderId::Qq,
        message: err.to_string(),
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn unavailable_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: ProviderId::Qq,
        message: err.to_string(),
        retryable: true,
        action: None,
        raw_message: None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::utils::cookie::Cookie;

    use super::{
        QqClient, login_refresh_request_for_device, merge_cookie, playlist_song_write_body,
    };

    #[test]
    fn get_sign_executes_the_bundled_javascript() {
        if option_env!("CSIGNER_LIB_FILENAME").is_none() {
            return;
        }
        crate::utils::cryptors::csigner::init().expect("csigner 初始化应成功");
        let data = json!({"comm":{"ct":24},"req_1":{"module":"test","method":"test","param":{}}});

        assert_eq!(
            QqClient::new().get_sign(&data).expect("calculate qq sign"),
            "zzcfcaa938yzk1nuourdgrzbse3gvchq0j1vk92298b96"
        );
    }

    #[test]
    fn playlist_song_write_uses_matching_request_key_and_method() {
        for method in ["AddSonglist", "DelSonglist"] {
            let body = playlist_song_write_body(method, 201, "0039MnYb0qxYhV");
            let request = &body["req_0"];

            assert_eq!(request["method"], method);
            assert_eq!(request["param"]["dirId"], 201);
            assert_eq!(
                request["param"]["v_songInfo"][0]["songMid"],
                "0039MnYb0qxYhV"
            );
        }
    }

    #[test]
    fn login_refresh_matches_the_native_qq_request_shape() {
        let cookie = Cookie::new(
            "tmeLoginType=2; psrf_qqopenid=openid; qm_keyst=music-key; expired_in=3600; \
             musicid=10001; refresh_key=refresh-key; psrf_qqaccess_token=access; \
             psrf_qqrefresh_token=refresh; appid=qqmusic",
        )
        .map;

        let (key, request, is_wechat) =
            login_refresh_request_for_device(&cookie, "DESKTOP".to_owned()).unwrap();

        assert_eq!(key, "music.login.LoginServer.Login");
        assert!(!is_wechat);
        assert_eq!(request[key]["module"], "music.login.LoginServer");
        assert_eq!(request[key]["method"], "Login");
        assert_eq!(request[key]["param"]["deviceName"], "DESKTOP");
        assert_eq!(request[key]["param"]["deviceType"], "Widnows");
        assert_eq!(request[key]["param"]["onlyNeedAccessToken"], 0);
        assert_eq!(request[key]["param"]["forceRefreshToken"], 0);
        assert_eq!(request[key]["param"]["appid"], 100497308);
        assert_eq!(request[key]["param"]["expired_in"], 3600);
    }

    #[test]
    fn login_refresh_matches_the_native_wechat_request_shape() {
        let cookie = Cookie::new(
            "tmeLoginType=1; qm_keyst=music-key; musicid=1152921505274451474; \
             refresh_key=refresh-key",
        )
        .map;

        let (key, request, is_wechat) =
            login_refresh_request_for_device(&cookie, "DESKTOP".to_owned()).unwrap();

        assert_eq!(key, "WXLoginByToken");
        assert!(is_wechat);
        assert_eq!(request[key]["module"], "music.login.LoginServer");
        assert_eq!(request[key]["method"], "Login");
        assert_eq!(request[key]["param"]["deviceName"], "DESKTOP");
        assert_eq!(request[key]["param"]["deviceType"], "Widnows");
        assert_eq!(request[key]["param"]["strAppid"], "wx48db31d50e334801");
        assert_eq!(request[key]["param"]["musicid"], 1152921505274451474u64);
        assert_eq!(request[key]["param"]["expired_in"], 0);
        assert_eq!(request[key]["param"]["access_token"], "");
        assert_eq!(request[key]["param"]["openid"], "");
        assert_eq!(request[key]["param"]["refresh_token"], "");
    }

    #[test]
    fn refreshed_cookie_replaces_session_values_and_keeps_unrelated_values() {
        let cookie = merge_cookie(
            Cookie::new("qqmusic_gkey=old-gkey; qm_keyst=old-key; refresh_key=old-refresh"),
            "qm_keyst=new-key; refresh_key=new-refresh; tmeLoginType=2",
        );

        let cookie = Cookie::new(&cookie).map;
        assert_eq!(cookie["qqmusic_gkey"], "old-gkey");
        assert_eq!(cookie["qm_keyst"], "new-key");
        assert_eq!(cookie["refresh_key"], "new-refresh");
        assert_eq!(cookie["tmeLoginType"], "2");
    }
}
