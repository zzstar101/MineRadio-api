use std::sync::Arc;

use anyhow::Context;
use reqwest::{
    Client,
    header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, ORIGIN, REFERER, USER_AGENT},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::{
    providers::{
        ProviderId, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
        qq::model::{
            QqAlbumDetailResp, QqAlbumListResp, QqLyricResp, QqPlaylistDetailResp,
            QqPlaylistList1Resp, QqPlaylistList2Resp, QqSearchResp, QqTrackDetailResp,
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
        let uin = qq_user_id_from_cookie_map(&parse_cookie(&cookie))?;

        *self.uin.write().await = Some(uin.clone());
        Some(uin)
    }

    pub async fn euin(&self) -> Option<String> {
        if let Some(euin) = self.euin.read().await.clone() {
            return Some(euin);
        }

        let cookie = self.current_cookie().await?;
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

    async fn set_euin_from_login_status(&self, body: &Value) {
        if let Some(euin) = qq_login_status_euin(body) {
            self.set_euin(euin).await;
        }
    }

    #[allow(dead_code)]
    pub fn get_sign(&self, payload: &Value) -> ProviderResult<String> {
        let payload = serde_json::to_string(payload).map_err(|err| unavailable_error(err))?;
        Ok(sign(&payload))
    }

    pub(super) async fn search(&self, keyword: &str, offset: u32, limit: u32) -> ProviderResult<QqSearchResp> {
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

    pub async fn smartbox_search(&self, keyword: &str, limit: u32) -> ProviderResult<Vec<Value>> {
        let body = self
            .get_json(
                "https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg",
                &[
                    ("key", keyword.to_owned()),
                    ("format", "json".to_owned()),
                    ("g_tk", "5381".to_owned()),
                ],
                Some("https://y.qq.com/"),
                self.current_cookie().await.as_deref(),
            )
            .await?;
        let list = body
            .get("data")
            .and_then(|value| value.get("song"))
            .and_then(|value| value.get("itemlist"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(limit as usize)
            .collect();
        Ok(list)
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

    pub async fn song_url(
        &self,
        song_mid: &str,
        _quality: &str,
        filename: &str,
    ) -> ProviderResult<Value> {
        let cookie = self.current_cookie().await;
        let cookie_map = parse_cookie(cookie.as_deref().unwrap_or_default());
        let uin = self.uin().await.unwrap_or_else(|| "0".to_owned());
        let auth = qq_playback_key_from_cookie_map(&cookie_map);
        self.post_form(
            "https://u.y.qq.com/cgi-bin/musicu.fcg",
            &json!({
                "-": "getplaysongvkey",
                "g_tk": "5381",
                "loginUin": uin,
                "hostUin": 0,
                "format": "json",
                "inCharset": "utf8",
                "outCharset": "utf-8",
                "notice": 0,
                "platform": "yqq.json",
                "needNewCode": 0,
                "data": serde_json::to_string(&json!({
                    "req_0": {
                        "module": "vkey.GetVkeyServer",
                        "method": "CgiGetVkey",
                        "param": {
                            "filename": [filename],
                            "guid": "2796982635",
                            "songmid": [song_mid],
                            "songtype": [0],
                            "uin": uin,
                            "loginflag": 1,
                            "platform": "20"
                        }
                    },
                    "comm": {
                        "uin": uin,
                        "format": "json",
                        "ct": 19,
                        "cv": 0,
                        "authst": auth
                    }
                })).unwrap_or_default()
            }),
            None,
            cookie.as_deref(),
            None,
        )
        .await
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

    pub async fn login_status_with_cookie(
        &self,
        user_id: &str,
        cookie: &str,
    ) -> ProviderResult<Value> {
        let body = self
            .get_json(
                "http://c.y.qq.com/rsc/fcgi-bin/fcg_get_profile_homepage.fcg",
                &[
                    ("cid", "205360838".to_owned()),
                    ("userid", user_id.to_owned()),
                    ("reqfrom", "1".to_owned()),
                ],
                None,
                Some(cookie),
            )
            .await?;
        self.set_euin_from_login_status(&body).await;
        Ok(body)
    }

    pub async fn vip_info_with_cookie(&self, user_id: &str, cookie: &str) -> ProviderResult<Value> {
        self.get_json(
            "https://u.y.qq.com/cgi-bin/musicu.fcg",
            &[
                ("format", "json".to_owned()),
                (
                    "data",
                    serde_json::to_string(&json!({
                        "getVipInfo": {
                            "module": "userInfo.VipQueryServer",
                            "method": "SRFVipQuery_V2",
                            "param": { "uin_list": [user_id] }
                        },
                        "getNickHead": {
                            "module": "userInfo.BaseUserInfoServer",
                            "method": "get_user_baseinfo_v2",
                            "param": { "vec_uin": [user_id] }
                        },
                        "getVipIcon": {
                            "module": "music.lvz.VipIconUiShowSvr",
                            "method": "GetVipIconUiV2",
                            "param": { "MusicID": user_id, "PID": 8 }
                        }
                    }))
                    .unwrap_or_default(),
                ),
            ],
            Some("https://y.qq.com/m/myservice/index.html"),
            Some(cookie),
        )
        .await
    }

    pub fn filename_for_quality(media_mid: &str, quality: &str) -> String {
        let (prefix, ext) = qq_quality_file(quality);
        format!("{prefix}{media_mid}{ext}")
    }

    pub fn has_playback_key(cookie: &str) -> bool {
        let cookie_map = parse_cookie(cookie);
        !qq_playback_key_from_cookie_map(&cookie_map)
            .trim()
            .is_empty()
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

    pub async fn user_collect_songlists(&self, uin: &str) -> ProviderResult<QqPlaylistList2Resp> {
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

    pub(super) async fn official_playlist_detail(
        &self,
        playlist_id: &str,
        limit: u32,
    ) -> ProviderResult<QqPlaylistDetailResp> {
        let disstid = playlist_id.trim().parse::<u64>().map_err(internal_error)?;
        let song_num = limit.clamp(1, 500);
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
                        "song_begin": 0,
                        "song_num": song_num,
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
        let euin = self.euin().await.unwrap_or_default();
        let body = json!({
            "req_0": {
                "method": "CgiGetAlbumFavInfo",
                "module": "music.musicasset.AlbumFavRead",
                "param": {
                    "euin": euin,
                    "offset": 0,
                    "size": 48
                }
            }
        });
        let cookie = self.current_cookie().await;
        self.get_model(
            "https://u.y.qq.com/cgi-bin/musicu.fcg",
            &body,
            Some("https://y.qq.com/"),
            cookie.as_deref(),
            None,
            "album_list",
        )
        .await
    }

    pub(super) async fn album_detail(&self, mid: &str, offset: u32, limit: u32) -> ProviderResult<QqAlbumDetailResp> {
        let body = json!({
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
        });
        let cookie = self.current_cookie().await;
        self.get_model(
            "https://u.y.qq.com/cgi-bin/musicu.fcg",
            &body,
            Some("https://y.qq.com/"),
            cookie.as_deref(),
            None,
            "album_detail",
        )
        .await
    }

    pub async fn add_song_to_playlist(
        &self,
        playlist_id: &str,
        track_mid: &str,
    ) -> ProviderResult<Value> {
        self.get_json(
            "https://c.y.qq.com/splcloud/fcgi-bin/fcg_music_add2songdir.fcg",
            &[
                ("g_tk", "5381".to_owned()),
                ("midlist", track_mid.to_owned()),
                ("typelist", "13".to_owned()),
                ("dirid", playlist_id.to_owned()),
                ("addtype", "".to_owned()),
                ("formsender", "4".to_owned()),
                ("r2", "0".to_owned()),
                ("r3", "1".to_owned()),
                ("utf8", "1".to_owned()),
            ],
            None,
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn logout(&self) -> ProviderResult<Value> {
        Ok(json!({ "ok": true }))
    }

    async fn get_json(
        &self,
        url: &str,
        query: &[(&str, String)],
        referer: Option<&str>,
        cookie: Option<&str>,
    ) -> ProviderResult<Value> {
        let mut request = self.http.get(url).query(query);
        request = request.headers(build_headers(referer, cookie, false)?);
        let response = request
            .send()
            .await
            .context("send qq upstream request")
            .map_err(unavailable_error)?;
        let text = response
            .text()
            .await
            .context("read qq upstream response")
            .map_err(unavailable_error)?;
        parse_json_like(&text)
    }

    async fn post_json_with_sign<T: DeserializeOwned>(
        &self,
        body: &Value,
        referer: Option<&str>,
        cookie: Option<&str>,
        action: &str,
    ) -> ProviderResult<T> {
        let sign = self.get_sign(body)?;
        let response = self
            .http
            .post("https://u.y.qq.com/cgi-bin/musics.fcg")
            .query(&[("sign", sign.as_str())])
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

    async fn post_form(
        &self,
        url: &str,
        body: &Value,
        referer: Option<&str>,
        cookie: Option<&str>,
        content_type: Option<&str>,
    ) -> ProviderResult<Value> {
        let headers = build_headers(referer, cookie, true)?;
        let mut request = self.http.post(url).headers(headers);
        if let Some(content_type) = content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        let response = request
            .form(
                &body
                    .as_object()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(key, value)| (key, value_to_form(value)))
                    .collect::<Vec<_>>(),
            )
            .send()
            .await
            .context("send qq upstream post request")
            .map_err(unavailable_error)?;
        let text = response
            .text()
            .await
            .context("read qq upstream post response")
            .map_err(unavailable_error)?;
        parse_json_like(&text)
    }

    async fn get_model<T: DeserializeOwned>(
        &self,
        url: &str,
        body: &Value,
        referer: Option<&str>,
        cookie: Option<&str>,
        content_type: Option<&str>,
        action: &str,
    ) -> ProviderResult<T> {
        let headers = build_headers(referer, cookie, true)?;
        let mut request = self.http.post(url).headers(headers);
        if let Some(content_type) = content_type {
            request = request.header(CONTENT_TYPE, content_type);
        }
        let response = request
            .json(body)
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

fn parse_json_like(text: &str) -> ProviderResult<Value> {
    let trimmed = text.trim();
    let cleaned = trimmed
        .trim_start_matches("callback(")
        .trim_start_matches("MusicJsonCallback(")
        .trim_start_matches("jsonCallback(")
        .trim_end_matches(')');
    serde_json::from_str(cleaned).map_err(internal_error)
}

fn value_to_form(value: Value) -> String {
    match value {
        Value::String(value) => value,
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null => String::new(),
        other => serde_json::to_string(&other).unwrap_or_default(),
    }
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

fn qq_user_id_from_cookie_map(
    cookie: &std::collections::HashMap<String, String>,
) -> Option<String> {
    let login_type = cookie
        .get("login_type")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or_default();
    let raw = if login_type == 2 {
        cookie
            .get("wxuin")
            .or_else(|| cookie.get("uin"))
            .or_else(|| cookie.get("p_uin"))
    } else {
        cookie
            .get("uin")
            .or_else(|| cookie.get("qqmusic_uin"))
            .or_else(|| cookie.get("wxuin"))
            .or_else(|| cookie.get("p_uin"))
    }?;
    let digits = raw
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then_some(digits)
}

fn qq_login_status_euin(body: &Value) -> Option<String> {
    let euin = body
        .get("data")
        .and_then(|data| data.get("creator"))
        .and_then(|creator| creator.get("encrypt_uin"))
        .and_then(|value| match value {
            Value::String(value) => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
        .map(|value| value.trim().to_owned())?;
    (!euin.is_empty()).then_some(euin)
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

fn qq_quality_file(quality: &str) -> (&'static str, &'static str) {
    match quality.trim().to_lowercase().as_str() {
        "flac" | "lossless" | "hires" | "sq" | "jymaster" => ("F000", ".flac"),
        "ape" => ("A000", ".ape"),
        "320" | "exhigh" | "high" | "hq" => ("M800", ".mp3"),
        "m4a" | "aac" => ("C400", ".m4a"),
        _ => ("M500", ".mp3"),
    }
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

    use super::{QqClient, parse_cookie, qq_user_id_from_cookie_map};

    #[test]
    fn cookie_user_id_is_normalized() {
        let cookie = parse_cookie("uin=o0012345; login_type=1");

        assert_eq!(
            qq_user_id_from_cookie_map(&cookie).as_deref(),
            Some("0012345")
        );
    }

    #[tokio::test]
    async fn login_status_caches_encrypt_uin() {
        let client = QqClient::new();
        client
            .set_euin_from_login_status(&json!({
                "data": { "creator": { "encrypt_uin": "12345" } }
            }))
            .await;

        assert_eq!(client.euin().await.as_deref(), Some("12345"));
    }

    #[test]
    fn get_sign_executes_the_bundled_javascript() {
        let data = json!({"comm":{"ct":24},"req_1":{"module":"test","method":"test","param":{}}});

        assert_eq!(
            QqClient::new().get_sign(&data).expect("calculate qq sign"),
            "zzcfcaa938yzk1nuourdgrzbse3gvchq0j1vk92298b96"
        );
    }
}
