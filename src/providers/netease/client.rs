use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use reqwest::{
    Client, Response,
    header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{
    providers::{
        ProviderId, ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    services::auth_session,
    utils::{encrypt_eapi, encrypt_weapi, generate_weapi_secret_key},
};

use super::model::{NeteaseAlbumDetailResp, NeteaseAlbumListResp};

const API_DOMAIN: &str = "https://interface.music.163.com";
const DOMAIN: &str = "https://music.163.com";
const UA_API_IPHONE: &str = "NeteaseMusic 9.0.90/5038 (iPhone; iOS 16.2; zh_CN)";
const UA_WEAPI_PC: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0";
const DEFAULT_APPVER: &str = "9.0.90";
const DEFAULT_CHANNEL: &str = "distribution";
const DEFAULT_OS: &str = "iPhone OS";
const DEFAULT_OSVER: &str = "16.2";

#[derive(Clone)]
pub struct NeteaseClient {
    http: Client,
}

#[derive(Clone, Debug)]
pub struct NeteaseClientResponse {
    pub body: Value,
    pub cookie: Option<String>,
}

impl NeteaseClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
        }
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

    pub async fn cloudsearch(&self, keyword: &str, limit: u32) -> ProviderResult<Value> {
        self.request_eapi(
            "/api/cloudsearch/pc",
            json!({
                "s": keyword,
                "type": 1,
                "limit": limit,
                "offset": 0,
                "total": true,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
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
        self.request_eapi(
            "/api/song/enhance/player/url/v1",
            body,
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn song_url(&self, id: &str, br: u32) -> ProviderResult<Value> {
        self.request_eapi(
            "/api/song/enhance/player/url",
            json!({
                "ids": format!("[\"{id}\"]"),
                "br": br,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn lyric_new(&self, id: &str) -> ProviderResult<Value> {
        self.request_eapi(
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
        )
        .await
    }

    pub async fn lyric(&self, id: &str) -> ProviderResult<Value> {
        self.request_eapi(
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
        )
        .await
    }

    pub async fn playlist_detail(&self, id: &str) -> ProviderResult<Value> {
        self.request_eapi(
            "/api/v6/playlist/detail",
            json!({
                "id": id,
                "n": 100000,
                "s": 8,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn user_playlist(&self, uid: &str, limit: u32) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/user/playlist",
            json!({
                "uid": uid,
                "limit": limit,
                "offset": 0,
                "includeVideo": true,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub(super) async fn album_list(&self) -> ProviderResult<NeteaseAlbumListResp> {
        let cookie = self.current_cookie().await;
        self.get_model(
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
        self.get_model(
            &format!("/api/v1/album/{id}"),
            json!({}),
            cookie.as_deref(),
            "album_detail",
        )
        .await
    }

    pub async fn dj_hot(&self, limit: u32, offset: u32) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/djradio/hot/v1",
            json!({
                "limit": limit,
                "offset": offset
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn dj_detail(&self, rid: &str) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/djradio/v2/get",
            json!({
                "id": rid
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn dj_program(
        &self,
        rid: &str,
        limit: u32,
        offset: u32,
        asc: bool,
    ) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/dj/program/byradio",
            json!({
                "radioId": rid,
                "limit": limit,
                "offset": offset,
                "asc": asc
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn dj_sublist(&self, limit: u32, offset: u32) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/djradio/get/subed",
            json!({
                "limit": limit,
                "offset": offset,
                "total": true
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn user_audio(&self, uid: &str) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/djradio/get/byuser",
            json!({
                "userId": uid
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn dj_paygift(&self, limit: u32, offset: u32) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/djradio/home/paygift/list",
            json!({
                "limit": limit,
                "offset": offset,
                "_nmclfl": 1
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn record_recent_voice(&self, limit: u32) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/play-record/voice/list",
            json!({
                "limit": limit
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn personalized(&self, limit: u32) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/personalized/playlist",
            json!({
                "limit": limit,
                "total": true,
                "n": 1000
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn recommend_resource(&self) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/v1/discovery/recommend/resource",
            json!({}),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn recommend_songs(&self) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/v3/discovery/recommend/songs",
            json!({}),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn login_status(&self) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/w/nuser/account/get",
            json!({ "e_r": false }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn vip_info(&self, uid: &str) -> ProviderResult<Value> {
        let uid = uid.trim();
        if uid.is_empty() {
            return Ok(json!({}));
        }

        let cookie = self.current_cookie().await;
        let client_v2 = self
            .request_weapi(
                "/api/music-vip-membership/client/vip/info",
                json!({ "userId": uid }),
                cookie.as_deref(),
            )
            .await;
        let legacy = self
            .request_weapi(
                "/api/music-vip-membership/front/vip/info",
                json!({ "userId": uid }),
                cookie.as_deref(),
            )
            .await;

        merge_vip_info(client_v2, legacy)
    }

    pub async fn logout(&self) -> ProviderResult<Value> {
        self.request_eapi(
            "/api/logout",
            json!({ "e_r": false }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn login_qr_key(
        &self,
        cookie: Option<&str>,
    ) -> ProviderResult<NeteaseClientResponse> {
        self.request_weapi_response("/api/login/qrcode/unikey", json!({ "type": 3 }), cookie)
            .await
    }

    pub async fn login_qr_check(
        &self,
        key: &str,
        cookie: Option<&str>,
    ) -> ProviderResult<NeteaseClientResponse> {
        self.request_weapi_response(
            "/api/login/qrcode/client/login",
            json!({
                "key": key,
                "type": 3
            }),
            cookie,
        )
        .await
    }

    pub async fn like(&self, id: &str, liked: bool) -> ProviderResult<Value> {
        self.request_weapi(
            "/api/radio/like",
            json!({
                "alg": "itembased",
                "trackId": id,
                "like": liked,
                "time": "3",
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn song_like_check(&self, ids: &[String]) -> ProviderResult<Value> {
        let track_ids = json!(ids).to_string();
        self.request_eapi(
            "/api/song/like/check",
            json!({
                "trackIds": track_ids,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn likelist(&self, uid: &str) -> ProviderResult<Value> {
        self.request_eapi(
            "/api/song/like/get",
            json!({
                "uid": uid,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn playlist_tracks(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> ProviderResult<Value> {
        let track_ids = json!([track_id]).to_string();
        self.request_eapi(
            "/api/playlist/manipulate/tracks",
            json!({
                "op": "add",
                "pid": playlist_id,
                "trackIds": track_ids,
                "imme": "true",
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    pub async fn playlist_track_add(
        &self,
        playlist_id: &str,
        track_id: &str,
    ) -> ProviderResult<Value> {
        let tracks = json!([{"type": 3, "id": track_id}]).to_string();
        self.request_weapi(
            "/api/playlist/track/add",
            json!({
                "id": playlist_id,
                "tracks": tracks,
                "e_r": false
            }),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    async fn request_weapi(
        &self,
        uri: &str,
        payload: Value,
        cookie: Option<&str>,
    ) -> ProviderResult<Value> {
        Ok(self
            .request_weapi_response(uri, payload, cookie)
            .await?
            .body)
    }

    async fn get_model<T: DeserializeOwned>(
        &self,
        uri: &str,
        payload: Value,
        cookie: Option<&str>,
        action: &str,
    ) -> ProviderResult<T> {
        let body = self.request_weapi(uri, payload, cookie).await?;
        let raw_message = body.to_string();
        serde_json::from_value(body).map_err(|err| ProviderError {
            code: ProviderErrorCode::InvalidResponse,
            provider: ProviderId::Netease,
            message: format!("decode netease {action} response: {err}"),
            retryable: false,
            action: Some(action.to_owned()),
            raw_message: Some(raw_message),
        })
    }

    async fn request_weapi_response(
        &self,
        uri: &str,
        payload: Value,
        cookie: Option<&str>,
    ) -> ProviderResult<NeteaseClientResponse> {
        let cookie_map = process_cookie_map(parse_cookie_header(cookie.unwrap_or_default()));
        let csrf = cookie_map.get("__csrf").cloned().unwrap_or_default();
        let mut body = payload.as_object().cloned().unwrap_or_default();
        body.insert("csrf_token".to_owned(), Value::String(csrf));
        let encrypted = encrypt_weapi(&Value::Object(body), Some(&generate_weapi_secret_key()))
            .map_err(|err| internal_error(format!("encrypt weapi payload: {err}")))?;

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA_WEAPI_PC));
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
        )
        .await
    }

    async fn request_eapi(
        &self,
        uri: &str,
        payload: Value,
        cookie: Option<&str>,
    ) -> ProviderResult<Value> {
        let cookie_map = parse_cookie_header(cookie.unwrap_or_default());
        let header = create_eapi_header(&cookie_map);
        let mut body = payload.as_object().cloned().unwrap_or_default();
        body.insert(
            "header".to_owned(),
            Value::Object(
                header
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect(),
            ),
        );
        let encrypted = encrypt_eapi(uri, crate::utils::EapiBody::Json(&Value::Object(body)))
            .map_err(|err| internal_error(format!("encrypt eapi payload: {err}")))?;

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(UA_API_IPHONE));
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        headers.insert(COOKIE, header_value(&header_cookie_string(&header))?);

        Ok(self
            .post_form_response(
                format!("{API_DOMAIN}/eapi/{}", uri.trim_start_matches("/api/")),
                headers,
                HashMap::from([("params".to_owned(), encrypted.params)]),
            )
            .await?
            .body)
    }

    async fn post_form_response(
        &self,
        url: String,
        headers: HeaderMap,
        form: HashMap<String, String>,
    ) -> ProviderResult<NeteaseClientResponse> {
        let response = self
            .http
            .post(url)
            .headers(headers)
            .form(&form)
            .send()
            .await
            .context("send netease upstream request")
            .map_err(|err| unavailable_error(err.to_string()))?;
        let cookie = cookie_from_response(&response);

        let status = response.status();
        let text = response
            .text()
            .await
            .context("read netease upstream response")
            .map_err(|err| unavailable_error(err.to_string()))?;
        let body = serde_json::from_str::<Value>(&text).map_err(|err| {
            unavailable_error(format!(
                "parse netease upstream response: {err}; body: {text}"
            ))
        })?;

        let code = body
            .get("code")
            .and_then(Value::as_i64)
            .unwrap_or(i64::from(status.as_u16()));
        if (200..300).contains(&status.as_u16())
            && matches!(code, 200 | 201 | 302 | 400 | 502 | 800 | 801 | 802 | 803)
        {
            return Ok(NeteaseClientResponse { body, cookie });
        }

        Err(ProviderError {
            code: match code {
                401 => ProviderErrorCode::LoginRequired,
                _ => ProviderErrorCode::Unavailable,
            },
            provider: ProviderId::Netease,
            message: body
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("netease upstream error")
                .to_owned(),
            retryable: code == 401,
            action: (code == 401).then(|| "login".to_owned()),
            raw_message: Some(body.to_string()),
        })
    }
}

impl Default for NeteaseClient {
    fn default() -> Self {
        Self::new()
    }
}

fn merge_vip_info(
    client_v2: ProviderResult<Value>,
    legacy: ProviderResult<Value>,
) -> ProviderResult<Value> {
    let mut body = serde_json::Map::new();

    match (client_v2, legacy) {
        (Ok(client_v2), Ok(legacy)) => {
            body.insert("vipInfoV2".to_owned(), client_v2);
            body.insert("vipInfo".to_owned(), legacy);
        }
        (Ok(client_v2), Err(_)) => {
            body.insert("vipInfoV2".to_owned(), client_v2);
        }
        (Err(_), Ok(legacy)) => {
            body.insert("vipInfo".to_owned(), legacy);
        }
        (Err(err), Err(_)) => return Err(err),
    }

    Ok(Value::Object(body))
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

fn create_eapi_header(cookie: &HashMap<String, String>) -> HashMap<String, String> {
    let mut header = HashMap::from([
        (
            "__csrf".to_owned(),
            cookie.get("__csrf").cloned().unwrap_or_default(),
        ),
        (
            "appver".to_owned(),
            cookie
                .get("appver")
                .cloned()
                .unwrap_or_else(|| DEFAULT_APPVER.to_owned()),
        ),
        ("buildver".to_owned(), format!("{}", unix_ms() / 1_000)),
        (
            "channel".to_owned(),
            cookie
                .get("channel")
                .cloned()
                .unwrap_or_else(|| DEFAULT_CHANNEL.to_owned()),
        ),
        ("deviceId".to_owned(), unique_seed()),
        (
            "os".to_owned(),
            cookie
                .get("os")
                .cloned()
                .unwrap_or_else(|| DEFAULT_OS.to_owned()),
        ),
        (
            "osver".to_owned(),
            cookie
                .get("osver")
                .cloned()
                .unwrap_or_else(|| DEFAULT_OSVER.to_owned()),
        ),
        ("requestId".to_owned(), format!("{}_0001", unix_ms())),
        ("resolution".to_owned(), "1920x1080".to_owned()),
        ("versioncode".to_owned(), "140".to_owned()),
    ]);

    if let Some(music_a) = cookie.get("MUSIC_A").cloned() {
        header.insert("MUSIC_A".to_owned(), music_a);
    }
    if let Some(music_u) = cookie.get("MUSIC_U").cloned() {
        header.insert("MUSIC_U".to_owned(), music_u);
    }

    header
}

fn header_cookie_string(header: &HashMap<String, String>) -> String {
    header
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(value)
            )
        })
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

fn cookie_from_response(response: &Response) -> Option<String> {
    let values = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(split_combined_set_cookie_header)
        .filter_map(cookie_kv_from_set_cookie)
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        Some(values.join(";"))
    }
}

fn split_combined_set_cookie_header(header: &str) -> Vec<String> {
    header
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

fn cookie_kv_from_set_cookie(header: String) -> Option<String> {
    let pair = header.split(';').next()?.trim();
    if pair.is_empty() || !pair.contains('=') {
        None
    } else {
        Some(pair.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{merge_vip_info, unavailable_error};
    use serde_json::json;

    #[test]
    fn vip_info_keeps_a_successful_fallback_response() {
        let body = merge_vip_info(
            Ok(json!({ "level": 7 })),
            Err(unavailable_error("down".to_owned())),
        )
        .unwrap();

        assert_eq!(body["vipInfoV2"]["level"], 7);
        assert!(body.get("vipInfo").is_none());
    }

    #[test]
    fn vip_info_returns_an_error_when_all_requests_fail() {
        let err = merge_vip_info(
            Err(unavailable_error("v2 down".to_owned())),
            Err(unavailable_error("legacy down".to_owned())),
        )
        .unwrap_err();

        assert_eq!(err.message, "v2 down");
    }
}
