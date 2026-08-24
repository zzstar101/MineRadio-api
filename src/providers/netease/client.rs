use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use reqwest::{
    Client,
    header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{
    auth_session,
    providers::{
        ProviderId, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
        netease::model::{
            NeteaseDailySongsResp, NeteaseFMResp, NeteaseIntelligenceResp,
            NeteasePlaylistDetailResp, NeteasePlaylistListResp, NeteaseRcmdPageResp,
            NeteaseVipInfoResp, RcmdM1SingleResp,
        },
    },
    utils::{
        cookie::Cookie, decrypt_eapi_response, encrypt_eapi, encrypt_weapi,
        generate_weapi_secret_key,
    },
};

use super::model::{
    NeteaseAlbumDetailResp, NeteaseAlbumListResp, NeteaseLoginStatusResp, NeteaseLyricResp,
    NeteaseLyricV1Resp, NeteaseSearchAlbumResp, NeteaseSearchPlaylistResp, NeteaseSearchTrackResp,
};

const API_DOMAIN: &str = "https://interfacepc.music.163.com";
const DOMAIN: &str = "https://music.163.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; WOW64) AppleWebKit/537.36 (KHTML, like Gecko) Safari/537.36 Chrome/91.0.4472.164 NeteaseMusicDesktop/3.1.34.205281";
const CFG: &str = "{\"IuRPVVmc3WWul9fT\":{\"version\":983040,\"appver\":\"3.1.34.205281\"}}";

#[derive(Clone)]
pub struct NeteaseClient {
    http: Client,
}

impl NeteaseClient {
    pub fn new() -> Self {
        Self::with_client(Client::new())
    }

    pub fn with_client(http: Client) -> Self {
        Self { http }
    }

    pub async fn current_cookie(&self) -> Option<String> {
        auth_session::get_provider_cookie(&ProviderId::Netease).await
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
                provider: ProviderId::Netease,
                message: "netease login required".to_owned(),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: None,
            });
        }
        Ok(())
    }

    pub async fn cloudsearch(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<Value> {
        self.eapi_model(
            "/api/cloudsearch/pc",
            json!({
                "s": keyword,
                "type": 1,
                "limit": limit,
                "offset": offset,
                "total": true,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "cloudsearch",
        )
        .await
    }

    pub(super) async fn search_track_modeled(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<NeteaseSearchTrackResp> {
        self.eapi_model(
            "/api/cloudsearch/pc",
            json!({
                "s": keyword,
                "type": 1,
                "limit": limit,
                "offset": offset,
                "total": true,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "search_track",
        )
        .await
    }

    pub(super) async fn search_album(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<NeteaseSearchAlbumResp> {
        self.eapi_model(
            "/api/v1/search/album/get",
            json!({
                "s": keyword,
                "limit": limit,
                "offset": offset,
                "queryCorrect": true,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "search_album",
        )
        .await
    }

    pub(super) async fn search_playlist(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<NeteaseSearchPlaylistResp> {
        self.eapi_model(
            "/api/v1/search/playlist/get",
            json!({
                "s": keyword,
                "limit": limit,
                "offset": offset,
                "queryCorrect": true,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "search_playlist",
        )
        .await
    }

    pub async fn song_url_v1(&self, id: &str, level: &str) -> ProviderResult<Value> {
        let mut body = json!({
            "ids": format!("[{id}]"),
            "level": level,
            "encodeType": "flac",
            "e_r": false
        });
        if level == "sky" {
            body["immerseType"] = Value::String("c51".to_owned());
        }
        self.eapi_model(
            "/api/song/enhance/player/url/v1",
            body,
            self.current_cookie().await.as_deref(),
            "song_url_v1",
        )
        .await
    }

    pub async fn song_url(&self, id: &str, br: u32) -> ProviderResult<Value> {
        self.eapi_model(
            "/api/song/enhance/player/url",
            json!({
                "ids": format!("[\"{id}\"]"),
                "br": br,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "song_url",
        )
        .await
    }

    pub(super) async fn lyric_new(&self, id: &str) -> ProviderResult<NeteaseLyricResp> {
        let v1: NeteaseLyricV1Resp = self
            .eapi_model(
                "/api/song/lyric/v1",
                json!({
                    "id": id,
                    "cp": false,
                    "tv": 0,
                    "lv": 0,
                    "rv": 0,
                    "kv": 0,
                    "yv": 0,
                    "ytv": 0,
                    "yrv": 0,
                    "e_r": false
                }),
                self.current_cookie().await.as_deref(),
                "lyric_new",
            )
            .await?;
        Ok(v1.into())
    }

    pub(super) async fn lyric(&self, id: &str) -> ProviderResult<NeteaseLyricResp> {
        self.eapi_model(
            "/api/song/lyric",
            json!({
                "id": id,
                "tv": -1,
                "lv": -1,
                "rv": -1,
                "kv": -1,
                "_nmclfl": 1,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "lyric",
        )
        .await
    }

    pub(super) async fn playlist_detail(
        &self,
        id: &str,
    ) -> ProviderResult<NeteasePlaylistDetailResp> {
        //v6接口不支持offset分页(n只截断track数), 一次取全量由adapter本地切片
        self.eapi_model(
            "/api/v6/playlist/detail",
            json!({
                "id": id,
                "n": 1000,
                "s": 0,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "playlist_detail",
        )
        .await
    }

    pub(super) async fn playlist_list(
        &self,
        uid: &str,
        limit: u32,
    ) -> ProviderResult<NeteasePlaylistListResp> {
        self.weapi_model(
            "/api/user/playlist",
            json!({
                "uid": uid,
                "limit": limit,
                "offset": 0,
                "includeVideo": true,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "playlist_list",
        )
        .await
    }

    pub(super) async fn album_list(&self) -> ProviderResult<NeteaseAlbumListResp> {
        let cookie = self.current_cookie().await;
        self.weapi_model(
            "/api/album/sublist",
            json!({
                "limit": 1000,
                "offset": 0,
                "total": true
            }),
            cookie.as_deref(),
            "album_list",
        )
        .await
    }

    pub(super) async fn album_detail(&self, id: &str) -> ProviderResult<NeteaseAlbumDetailResp> {
        let cookie = self.current_cookie().await;
        self.weapi_model(
            &format!("/api/v1/album/{id}"),
            json!({}),
            cookie.as_deref(),
            "album_detail",
        )
        .await
    }

    pub async fn dj_hot(&self, limit: u32, offset: u32) -> ProviderResult<Value> {
        self.weapi_model(
            "/api/djradio/hot/v1",
            json!({
                "limit": limit,
                "offset": offset
            }),
            self.current_cookie().await.as_deref(),
            "dj_hot",
        )
        .await
    }

    pub async fn dj_detail(&self, rid: &str) -> ProviderResult<Value> {
        self.weapi_model(
            "/api/djradio/v2/get",
            json!({
                "id": rid
            }),
            self.current_cookie().await.as_deref(),
            "dj_detail",
        )
        .await
    }

    pub async fn dj_program(&self, rid: &str, limit: u32, offset: u32) -> ProviderResult<Value> {
        self.weapi_model(
            "/api/dj/program/byradio",
            json!({
                "radioId": rid,
                "limit": limit,
                "offset": offset,
                "asc": false
            }),
            self.current_cookie().await.as_deref(),
            "dj_program",
        )
        .await
    }

    pub async fn dj_sublist(&self, limit: u32, offset: u32) -> ProviderResult<Value> {
        self.weapi_model(
            "/api/djradio/get/subed",
            json!({
                "limit": limit,
                "offset": offset,
                "total": true
            }),
            self.current_cookie().await.as_deref(),
            "dj_sublist",
        )
        .await
    }

    pub async fn user_audio(&self, uid: &str) -> ProviderResult<Value> {
        self.weapi_model(
            "/api/djradio/get/byuser",
            json!({
                "userId": uid
            }),
            self.current_cookie().await.as_deref(),
            "user_audio",
        )
        .await
    }

    pub async fn dj_paygift(&self, limit: u32, offset: u32) -> ProviderResult<Value> {
        self.weapi_model(
            "/api/djradio/home/paygift/list",
            json!({
                "limit": limit,
                "offset": offset,
                "_nmclfl": 1
            }),
            self.current_cookie().await.as_deref(),
            "dj_paygift",
        )
        .await
    }

    pub async fn record_recent_voice(&self, limit: u32) -> ProviderResult<Value> {
        self.weapi_model(
            "/api/play-record/voice/list",
            json!({
                "limit": limit
            }),
            self.current_cookie().await.as_deref(),
            "record_recent_voice",
        )
        .await
    }

    pub async fn login_status(&self) -> ProviderResult<NeteaseLoginStatusResp> {
        self.weapi_model(
            "/api/w/nuser/account/get",
            json!({ "e_r": false }),
            self.current_cookie().await.as_deref(),
            "login_status",
        )
        .await
    }

    pub(super) async fn vip_info(&self, uid: &str) -> ProviderResult<NeteaseVipInfoResp> {
        let uid = uid.trim();
        if uid.is_empty() {
            return Err(unavailable_error("vip_info"));
        }

        self.weapi_model(
            "/api/music-vip-membership/front/vip/info",
            json!({ "userId": uid }),
            self.current_cookie().await.as_deref(),
            "vip_info",
        )
        .await
    }

    pub async fn logout(&self) -> ProviderResult<Value> {
        self.eapi_model(
            "/api/logout",
            json!({ "e_r": false }),
            self.current_cookie().await.as_deref(),
            "logout",
        )
        .await
    }

    pub async fn like(&self, id: &str, liked: bool) -> ProviderResult<Value> {
        self.weapi_model(
            "/api/radio/like",
            json!({
                "alg": "itembased",
                "trackId": id,
                "like": liked,
                "time": "3",
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "like",
        )
        .await
    }

    pub async fn song_like_check(&self, ids: &[String]) -> ProviderResult<Value> {
        let track_ids = json!(ids).to_string();
        self.eapi_model(
            "/api/song/like/check",
            json!({
                "trackIds": track_ids,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "song_like_check",
        )
        .await
    }

    pub async fn likelist(&self, uid: &str) -> ProviderResult<Value> {
        self.eapi_model(
            "/api/song/like/get",
            json!({
                "uid": uid,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "likelist",
        )
        .await
    }

    pub async fn playlist_tracks(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> ProviderResult<Value> {
        let track_ids = json!([track_id]).to_string();
        self.eapi_model(
            "/api/playlist/manipulate/tracks",
            json!({
                "op": "add",
                "pid": playlist_id,
                "trackIds": track_ids,
                "imme": "true",
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "playlist_tracks",
        )
        .await
    }

    pub async fn playlist_track_add(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> ProviderResult<Value> {
        let tracks = json!([{"type": 3, "id": track_id}]).to_string();
        self.weapi_model(
            "/api/playlist/track/add",
            json!({
                "id": playlist_id,
                "tracks": tracks,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
            "playlist_track_add",
        )
        .await
    }

    pub(super) async fn recommendation_module1(&self) -> ProviderResult<RcmdM1SingleResp> {
        let now = chrono::Local::now();
        let time = now.format("%Y-%m-%d %H:%M:%S").to_string();

        self.eapi_model(
            "/api/pc/daily/rcmd/block",
            json!({
                "clientTime": time,
                "e_r": true,
            }),
            self.current_cookie().await.as_deref(),
            "recommendation_module1",
        )
        .await
    }

    pub(super) async fn recommendation_page(&self) -> ProviderResult<NeteaseRcmdPageResp> {
        let now = chrono::Local::now();
        let time = now.format("%Y-%m-%d %H:%M:%S").to_string();

        self.eapi_model(
            "/api/pc/page/rcmd/resource/show",
            json!({
                "pageCode": "PC_RECOMMEND_HOME",
                "isFirstScreen": "true",
                "cursor": "0",
                "extJson": "",
                "blockCodeOrderList": "",
                "blockRequestParam": "{\"PC_HOMEPAGE_DAILY_MIX\":{\"clientTime\":\"".to_owned() + &time + "\"},\"HOMEPAGE_BLOCK_PLAYLIST_RCMD\":{\"cursor\":\"{\\\"offset\\\":0,\\\"blockCodeOrderList\\\":[\\\"HOMEPAGE_BLOCK_PLAYLIST_RCMD\\\"]}\",\"extInfo\":\"{\\\"abInfo\\\":{\\\"hp-new-homepageV3.1\\\":\\\"t3\\\"}}\",\"newStyle\":true},\"PC_HOMEPAGE_BANNER_BLOCK\":{\"clientType\":\"pc\"},\"PC_HOMEPAGE_TOPLIST\":{},\"HOMEPAGE_BLOCK_ALL_LISTEN\":{\"cursor\":\"{\\\"offset\\\":0,\\\"blockCodeOrderList\\\":[\\\"HOMEPAGE_BLOCK_ALL_LISTEN\\\"]}\",\"extInfo\":\"{\\\"abInfo\\\":{\\\"hp-new-homepageV3.1\\\":\\\"t3\\\"}}\",\"newStyle\":true},\"PC_HOMEPAGE_RECENT_LISTEN_BLOCK\":{},\"HOMEPAGE_BLOCK_RED_SIMILAR_SONG\":{\"cursor\":\"{\\\"offset\\\":0,\\\"blockCodeOrderList\\\":[\\\"HOMEPAGE_BLOCK_RED_SIMILAR_SONG\\\"]}\",\"extInfo\":\"{\\\"abInfo\\\":{\\\"hp-new-homepageV3.1\\\":\\\"t3\\\"}}\",\"newStyle\":true},\"CUSTOMIZE_PLAYLIST_MGC\":{\"newStyle\":true},\"HOMPAGE_BLOCK_VIP_RCMD\":{\"cursor\":\"{\\\"offset\\\":0,\\\"blockCodeOrderList\\\":[\\\"HOMPAGE_BLOCK_VIP_RCMD\\\"]}\",\"extInfo\":\"{\\\"abInfo\\\":{\\\"hp-new-homepageV3.1\\\":\\\"t3\\\"}}\",\"newStyle\":true},\"HOMEPAGE_BLOCK_STYLE_RCMD\":{\"cursor\":\"{\\\"offset\\\":0,\\\"blockCodeOrderList\\\":[\\\"HOMEPAGE_BLOCK_STYLE_RCMD\\\"]}\",\"extInfo\":\"{\\\"abInfo\\\":{\\\"hp-new-homepageV3.1\\\":\\\"t3\\\"}}\",\"newStyle\":true},\"HOMEPAGE_MUSIC_PODCAST_RCMD_BLOCK\":{\"cursor\":\"{\\\"offset\\\":0,\\\"blockCodeOrderList\\\":[\\\"HOMEPAGE_MUSIC_PODCAST_RCMD_BLOCK\\\"]}\",\"extInfo\":\"{\\\"abInfo\\\":{\\\"hp-new-homepageV3.1\\\":\\\"t3\\\"}}\",\"newStyle\":true},\"PC_HOME_PAGE_PERSONAL_RCMD_VOICE\":{\"limit\":9},\"PC_HOMEPAGE_VOICEBOOK_RCMD\":{\"pageCode\":\"PC_HOMEPAGE_PODCAST\",\"blockCode\":\"PC_HOMEPAGE_VOICEBOOK_RCMD\",\"extInfo\":\"{\\\"position\\\":\\\"homePage\\\"}\"}}",
                "e_r": true
            }),
            self.current_cookie().await.as_deref(),
            "recommendation_page",
        )
        .await
    }
    ///"每日推荐"接口
    pub(super) async fn daily_songs(&self) -> ProviderResult<NeteaseDailySongsResp> {
        self.eapi_model(
            "/api/v3/discovery/recommend/songs",
            json!({
                "limit": "30",
                "e_r": true
            }),
            self.current_cookie().await.as_deref(),
            "daily_songs",
        )
        .await
    }
    ///除了"每日推荐"其他的每日推荐接口
    pub(super) async fn daily_songs2(&self, list: &str) -> ProviderResult<NeteaseDailySongsResp> {
        let mut map: serde_json::Map<String, Value> = list
            .split('&')
            .filter_map(|item| {
                let (key, value) = item.split_once('=')?;
                Some((key.to_owned(), Value::String(value.to_owned())))
            })
            .collect();

        map.insert("e_r".to_string(), Value::Bool(true));

        self.eapi_model(
            "/api/homepage/category/daily/song/list",
            Value::Object(map),
            self.current_cookie().await.as_deref(),
            "daily_songs",
        )
        .await
    }
    ///"私人漫游"
    pub(super) async fn personal_fm(&self) -> ProviderResult<NeteaseFMResp> {
        self.eapi_model(
            "/api/v1/radio/get",
            json!({
                "imageFm": "0",
                "e_r": true
            }),
            self.current_cookie().await.as_deref(),
            "personal_fm",
        )
        .await
    }
    ///"心动模式", 使用预提供的pid和tid
    pub(super) async fn star_mode(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> ProviderResult<NeteaseIntelligenceResp> {
        self.eapi_model(
            "/api/playmode/intelligence/list",
            json!({
              "playlistId": playlist_id,
              "songId": track_id,
              "type": "fromPlayOne",
              "startMusicId": track_id,
              "count": "1",
              "e_r": true
            }),
            self.current_cookie().await.as_deref(),
            "intelligence",
        )
        .await
    }

    async fn eapi_model<T: DeserializeOwned>(
        &self,
        uri: &str,
        payload: Value,
        cookie: Option<&str>,
        action: &str,
    ) -> ProviderResult<T> {
        let response_encrypted = payload.get("e_r").and_then(Value::as_bool).unwrap_or(false);
        let mut body = payload.as_object().cloned().unwrap_or_default();
        let cookie = if let Some(cookie) = cookie {
            let c = Cookie::new(cookie);
            let v = vec![
                ("clientSign", ""),
                ("os", "pc"),
                ("appver", "3.1.34.205281"),
                ("deviceId", ""),
                (
                    "osver",
                    "Microsoft-Windows-11-Professional-build-114514-64bit",
                ),
            ];
            //find_or::<Value> will return Null.
            let mut header: HashMap<String, Value> = v
                .into_iter()
                .map(|(a, b)| {
                    (
                        a.to_owned(),
                        serde_json::json!(c.find_or::<String>(a, b.into())),
                    )
                })
                .collect();
            header.insert("requestId".to_owned(), json!(0));
            body.insert(
                "header".to_owned(),
                json!(serde_json::to_string(&header).unwrap_or_default()),
            );
            &format!(
                "{}; _ntes_nnid={},{}",
                cookie.trim_end_matches(";"),
                c.find_or_default::<String>("_ntes_nuid"),
                chrono::Utc::now().timestamp_millis()
            )
        } else {
            ""
        };

        let encrypted = encrypt_eapi(uri, crate::utils::EapiBody::Json(&Value::Object(body)))
            .map_err(|err| internal_error(format!("encrypt eapi payload: {err}")))?;

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA));

        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );

        headers.insert("mconfig-info", HeaderValue::from_static(CFG));

        headers.insert(COOKIE, header_value(&cookie)?);

        Ok(self
            .post_form_response(
                format!("{API_DOMAIN}/eapi/{}", uri.trim_start_matches("/api/")),
                headers,
                HashMap::from([("params".to_owned(), encrypted.params)]),
                response_encrypted,
                action,
            )
            .await?)
    }

    async fn weapi_model<T: DeserializeOwned>(
        &self,
        uri: &str,
        payload: Value,
        cookie: Option<&str>,
        action: &str,
    ) -> ProviderResult<T> {
        let cookie_map = process_cookie_map(parse_cookie_header(cookie.unwrap_or_default()));
        let csrf = cookie_map.get("__csrf").cloned().unwrap_or_default();
        let mut body = payload.as_object().cloned().unwrap_or_default();
        body.insert("csrf_token".to_owned(), Value::String(csrf));
        let encrypted = encrypt_weapi(&Value::Object(body), Some(&generate_weapi_secret_key()))
            .map_err(|err| internal_error(format!("encrypt weapi payload: {err}")))?;

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA));
        headers.insert(REFERER, HeaderValue::from_static(DOMAIN));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        if !cookie_map.is_empty() {
            headers.insert(COOKIE, header_value(&cookie_map_to_string(&cookie_map))?);
        }

        self.post_form_response(
            format!("{DOMAIN}/weapi/{}", uri.trim_start_matches("/api/")),
            headers,
            HashMap::from([
                ("params".to_owned(), encrypted.params),
                ("encSecKey".to_owned(), encrypted.enc_sec_key),
            ]),
            false,
            action,
        )
        .await
    }

    async fn post_form_response<T: DeserializeOwned>(
        &self,
        url: String,
        headers: HeaderMap,
        form: HashMap<String, String>,
        response_encrypted: bool,
        action: &str,
    ) -> ProviderResult<T> {
        let response = self
            .http
            .post(url)
            .headers(headers)
            .form(&form)
            .send()
            .await
            .context("send netease upstream request")
            .map_err(|err| unavailable_error(err.to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .context("read netease upstream response")
            .map_err(|err| unavailable_error(err.to_string()))?;
        let raw_body = String::from_utf8_lossy(&bytes).into_owned();
        let body = if response_encrypted {
            let decrypted = decrypt_eapi_response(&bytes, false).map_err(|err| {
                unavailable_error(format!("decrypt netease {action} eapi response: {err}"))
            })?;
            serde_json::from_slice::<T>(&decrypted).map_err(|err| {
                unavailable_error(format!("parse netease {action} eapi response: {err}"))
            })?
        } else {
            serde_json::from_slice::<T>(&bytes).map_err(|err| {
                unavailable_error(format!(
                    "parse netease {action} upstream response: {err}; body: {raw_body}"
                ))
            })?
        };
        if status.is_success() {
            return Ok(body);
        }

        Err(ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: ProviderId::Netease,
            message: format!("netease upstream http {}", status.as_u16()),
            retryable: true,
            action: None,
            raw_message: Some(raw_body),
        })
    }
}

impl Default for NeteaseClient {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_cookie_header(cookie: &str) -> HashMap<String, String> {
    cookie
        .split(';')
        .filter_map(|segment| {
            let (name, value) = segment.trim().split_once('=')?;
            let key = name.trim();
            let value = value.trim();
            if key.is_empty() || value.is_empty() {
                None
            } else {
                Some((key.to_owned(), value.to_owned()))
            }
        })
        .collect()
}

fn process_cookie_map(mut cookie: HashMap<String, String>) -> HashMap<String, String> {
    let seed = unique_seed();
    cookie
        .entry("__remember_me".to_owned())
        .or_insert_with(|| "true".to_owned());
    cookie
        .entry("_ntes_nuid".to_owned())
        .or_insert_with(|| seed.clone());
    cookie
        .entry("_ntes_nnid".to_owned())
        .or_insert_with(|| format!("{seed},{}", unix_ms()));
    cookie
        .entry("WEVNSM".to_owned())
        .or_insert_with(|| "1.0.0".to_owned());
    cookie
        .entry("WNMCID".to_owned())
        .or_insert_with(|| format!("{}.{}.01.0", &seed[..6.min(seed.len())], unix_ms()));
    cookie
        .entry("appver".to_owned())
        .or_insert_with(|| "3.1.17.204416".to_owned());
    cookie
        .entry("channel".to_owned())
        .or_insert_with(|| "netease".to_owned());
    cookie
        .entry("os".to_owned())
        .or_insert_with(|| "pc".to_owned());
    cookie
        .entry("osver".to_owned())
        .or_insert_with(|| "Microsoft-Windows-10-Professional-build-19045-64bit".to_owned());
    cookie
}

fn cookie_map_to_string(cookie: &HashMap<String, String>) -> String {
    cookie
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn header_value(value: &str) -> ProviderResult<HeaderValue> {
    HeaderValue::from_str(value).map_err(|err| internal_error(format!("build header: {err}")))
}

fn unique_seed() -> String {
    format!("netease{:x}", unix_ms())
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default()
}

fn internal_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Internal,
        provider: ProviderId::Netease,
        message: err.to_string(),
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn unavailable_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: ProviderId::Netease,
        message: err.to_string(),
        retryable: true,
        action: None,
        raw_message: None,
    }
}
