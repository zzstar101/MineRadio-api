use std::sync::Arc;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use reqwest::{
    Client,
    header::{COOKIE, HeaderMap, HeaderValue, ORIGIN, RANGE, REFERER, USER_AGENT},
};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::providers::qq::model::{QqLoginStatusResp, QqSongUrlResp, QqVipIconResp};
use crate::utils::cryptors::qq::get_guid;
use crate::{
    providers::{
        ProviderId, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
        qq::model::{
            QqAlbumDetailResp, QqAlbumListResp, QqLyricResp, QqMultiSearchResp,
            QqPlaylistDetailResp, QqPlaylistList1Resp, QqPlaylistList2Resp,
            QqPlaylistSongWriteResp, QqSearchResp, QqTrackDetailResp,
        },
    },
    services::auth_session,
    utils::cryptors::qq::sign,
};

const UA: &str = "Mozilla/5.0";

#[derive(Clone, Default)]
pub struct QqClient {
    http: Client,
    uin: Arc<RwLock<Option<String>>>,
    euin: Arc<RwLock<Option<String>>>,
}

impl QqClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            uin: Arc::new(RwLock::new(None)),
            euin: Arc::new(RwLock::new(None)),
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
        let uin = uin_from_cookie_map(&parse_cookie(&cookie))?;

        *self.uin.write().await = Some(uin.clone());
        Some(uin)
    }

    pub async fn euin(&self) -> Option<String> {
        if let Some(euin) = self.euin.read().await.clone() {
            return Some(euin);
        }

        let cookie = self.current_cookie().await?;
        if let Some(euin) = euin_from_cookie_map(&parse_cookie(&cookie)) {
            self.set_euin(euin.clone()).await;
            return Some(euin);
        }

        let uin = self.uin().await?;
        let _ = self.login_status_with_cookie(&uin, &cookie).await;
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
        Ok(sign(&payload))
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

    /// 搜索专辑（DoSearchForQQMusicDesktop, search_type=2）
    pub(super) async fn search_album(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QqMultiSearchResp> {
        let page_num = (offset / limit.max(1) + 1).to_string();
        self.post_json_with_sign(
            &serde_json::json!({
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
        )
        .await
    }

    /// 搜索歌单（DoSearchForQQMusicDesktop, search_type=3）
    pub(super) async fn search_playlist(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QqMultiSearchResp> {
        let page_num = (offset / limit.max(1) + 1).to_string();
        self.post_json_with_sign(
            &serde_json::json!({
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
        )
        .await
    }

    /// 搜索单曲（DoSearchForQQMusicDesktop, search_type=0）—— 备用接口，数据比旧 shc.y.qq.com 更丰富
    pub(super) async fn multi_search_track(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<QqMultiSearchResp> {
        let page_num = (offset / limit.max(1) + 1).to_string();
        self.post_json_with_sign(
            &serde_json::json!({
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
        )
        .await
    }

    pub(super) async fn song_detail(&self, song_mid: &str) -> ProviderResult<QqTrackDetailResp> {
        self.post_json_with_sign(
            &json!({
                "req_0": {
                    "method": "get_song_detail_yqq",
                    "module": "music.pf_song_detail_svr",
                    "param": { "song_mid": song_mid }
                }
            }),
            None,
            self.current_cookie().await.as_deref(),
            "song_detail",
        )
        .await
    }

    pub(super) async fn song_url(
        &self,
        song_mid: &str,
        filenames: String,
    ) -> ProviderResult<QqSongUrlResp> {
        let cookie = self.current_cookie().await;
        let cookie_map = parse_cookie(cookie.as_deref().unwrap_or_default());
        let uin = self.uin().await.unwrap_or_else(|| "0".to_owned());
        let auth = qq_playback_key_from_cookie_map(&cookie_map);
        self.post_json_with_sign(
            &json!({
                "req_0": {
                    "module": "music.vkey.GetVkey",
                    "method": "UrlGetVkey",
                    "param": {
                        "guid": get_guid(),
                        "uin": uin,
                        "downloadfrom": 1,
                        "ctx": 1,
                        "referer": "y.qq.com",
                        "scene": 0,
                        "songtype": [1],
                        "songmid": [song_mid],
                        "filename": [filenames],
                    },
                },
                "comm": {
                    "uin": uin,
                    "format": "json",
                    "ct": 19,
                    "cv": 0,
                    "authst": auth
                }
            }),
            None,
            cookie.as_deref(),
            "song_url",
        )
        .await
    }

    pub async fn probe_playback_url(&self, url: &str, timeout: Duration) -> bool {
        let timeout = timeout.max(Duration::from_millis(1));
        let headers = match build_headers(Some("https://y.qq.com/"), None, false) {
            Ok(headers) => headers,
            Err(_) => return false,
        };
        let mut response = match self
            .http
            .get(url)
            .headers(headers)
            .header(RANGE, "bytes=0-8191")
            .timeout(timeout)
            .send()
            .await
        {
            Ok(response) if matches!(response.status().as_u16(), 200 | 206) => response,
            _ => return false,
        };
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if content_type.contains("text/html")
            || content_type.contains("application/json")
            || content_type.contains("application/xml")
            || content_type.contains("text/plain")
        {
            return false;
        }

        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::with_capacity(8192);
        while bytes.len() < 8192 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let chunk = match tokio::time::timeout(remaining, response.chunk()).await {
                Ok(Ok(Some(chunk))) => chunk,
                _ => break,
            };
            let remaining_len = 8192 - bytes.len();
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining_len)]);
        }
        bytes.len() >= 512
    }

    pub(super) async fn lyric(&self, song_mid: &str) -> ProviderResult<QqLyricResp> {
        self.post_json_with_sign(
            &json!({"req_0": {
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
                &format!("http://c.y.qq.com/rsc/fcgi-bin/fcg_get_profile_homepage.fcg?cid=205360838&ct=20&cv=2230&userid={}&reqfrom=1&reqtype=0", user_id),
                None,
                Some(cookie),
                "login_status"
            )
            .await?;
        self.set_euin(body.encrypted_uin()).await;
        Ok(body)
    }

    pub(super) async fn vip_info_with_cookie(
        &self,
        user_id: &str,
        cookie: &str,
    ) -> ProviderResult<QqVipIconResp> {
        self.post_json_with_sign(
            &json!({
                "getVipIcon": {
                    "module": "music.lvz.VipIconUiShowSvr",
                    "method": "GetVipIconUiV2",
                    "param": { "Encuin": user_id, "PID": 8 }
                }
            }),
            Some("https://y.qq.com/m/myservice/index.html"),
            Some(cookie),
            "vip_info",
        )
        .await
    }

    pub(super) async fn user_songlists(&self, euin: &str) -> ProviderResult<QqPlaylistList1Resp> {
        self.post_json_with_sign(
            &json!({
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
        )
        .await
    }

    pub(super) async fn user_collect_songlists(
        &self,
        uin: &str,
    ) -> ProviderResult<QqPlaylistList2Resp> {
        self.post_json_with_sign(
            &json!({
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
            &json!({
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
        )
        .await
    }

    pub(super) async fn album_list(&self) -> ProviderResult<QqAlbumListResp> {
        self.post_json_with_sign(
            &json!({
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
            &json!({
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
            &playlist_song_write_body(method, dir_id, track_id),
            None,
            self.current_cookie().await.as_deref(),
            "playlist_song_write",
        )
        .await
    }

    pub async fn logout(&self) -> ProviderResult<IgnoredAny> {
        self.post_json_with_sign(
            &json!({
                "music.login.LoginServer.Logout": {
                    "method": "Logout",
                    "module": "music.login.LoginServer",
                    "param": {}
                }
            }),
            None,
            self.current_cookie().await.as_deref(),
            "logout",
        )
        .await
    }

    async fn post_json_with_sign<T: DeserializeOwned>(
        &self,
        body: &Value,
        referer: Option<&str>,
        cookie: Option<&str>,
        action: &str,
    ) -> ProviderResult<T> {
        let sign = self.get_sign(body)?;
        let now = SystemTime::now();
        let since_epoch = now
            .duration_since(UNIX_EPOCH)
            .expect("系统时间早于 UNIX 纪元");
        let millis = since_epoch.as_secs() * 1000 + since_epoch.subsec_millis() as u64;

        let response = self
            .http
            .post("https://u.y.qq.com/cgi-bin/musics.fcg")
            .query(&[("sign", sign.as_str()), ("_", &millis.to_string())])
            .headers(build_headers(referer, cookie, false)?)
            .json(&body)
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

fn parse_cookie(cookie: &str) -> std::collections::HashMap<String, String> {
    cookie
        .split(';')
        .filter_map(|segment| {
            let (name, value) = segment.trim().split_once('=')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn uin_from_cookie_map(cookie: &std::collections::HashMap<String, String>) -> Option<String> {
    //login_type 微信1 qq2 qq音乐3
    let login_type = cookie
        .get("login_type")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_default();
    let raw = if login_type != 1 {
        cookie.get("uin")
    } else {
        cookie
            .get("qqmusic_uin")
            .or_else(|| cookie.get("wxuin"))
            .or_else(|| cookie.get("p_uin"))
            .or_else(|| cookie.get("uin"))
    }?;
    let digits = raw
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then_some(digits)
}

fn euin_from_cookie_map(cookie: &std::collections::HashMap<String, String>) -> Option<String> {
    //login_type 微信1 qq2 qq音乐3
    let login_type = cookie
        .get("login_type")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_default();
    if login_type != 1 {
        cookie.get("euin")
    } else {
        cookie.get("encrypt_uin").or_else(|| cookie.get("euin"))
    }
    .map(|e| e.to_owned())
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

    use super::{QqClient, parse_cookie, playlist_song_write_body, uin_from_cookie_map};

    #[test]
    fn cookie_user_id_is_normalized() {
        let cookie = parse_cookie("uin=o0012345; login_type=1");

        assert_eq!(uin_from_cookie_map(&cookie).as_deref(), Some("0012345"));
    }

    #[test]
    fn get_sign_executes_the_bundled_javascript() {
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
}
