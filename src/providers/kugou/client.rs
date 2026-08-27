#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use md5::{Digest, Md5};
use reqwest::{Client, Method, Response};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::auth_session;
use crate::providers::{
    ProviderId, ProviderResult,
    error::{ProviderError, ProviderErrorCode},
};

use super::model::{
    KugouAddSongRequest, KugouAuth, KugouCollectionResp, KugouDeleteSongRequest, KugouLyricResp,
    KugouLyricSearchResp, KugouPlaylistListRequest, KugouPlaylistTracksRequest,
};

const GATEWAY_URL: &str = "https://gateway.kugou.com";
const APP_ID: &str = "1005";
const CLIENT_VERSION: &str = "20489";
const USER_AGENT: &str = "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi";
const H5_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const WEB_SIGNATURE_SALT: &str = "NVPh5oo715z5DIWAeQlhMDsWXXQV4hwt";
const ANDROID_SIGNATURE_SALT: &str = "OIlwieks28dk2k092lksi2UIkp";
const REGISTER_SIGNATURE_SALT: &str = "1014";
const SIGN_KEY_SALT: &str = "57ae12eb6890223e355ccfcb74edf70d";

pub type KugouCookie = BTreeMap<String, String>;
pub type KugouParams = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Default)]
pub enum KugouSignature {
    #[default]
    Android,
    Web,
    H5,
    Register,
}

#[derive(Clone, Debug)]
pub enum KugouRequestBody {
    Json(Value),
    Text(String),
    Bytes(Vec<u8>),
}

impl KugouRequestBody {
    fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Json(value) => value.to_string().into_bytes(),
            Self::Text(value) => value.as_bytes().to_vec(),
            Self::Bytes(value) => value.clone(),
        }
    }

    fn content_type(&self) -> Option<&'static str> {
        match self {
            Self::Json(_) => Some("application/json"),
            Self::Text(_) | Self::Bytes(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct KugouRequest {
    pub method: Method,
    pub path: String,
    pub base_url: Option<String>,
    pub params: KugouParams,
    pub body: Option<KugouRequestBody>,
    pub headers: BTreeMap<String, String>,
    pub signature: KugouSignature,
    pub cookie: KugouCookie,
    pub encrypt_key: bool,
    pub clear_default_params: bool,
    pub skip_signature: bool,
}

impl KugouRequest {
    pub fn new(method: Method, path: impl Into<String>) -> Self {
        Self {
            method,
            path: path.into(),
            base_url: None,
            params: KugouParams::new(),
            body: None,
            headers: BTreeMap::new(),
            signature: KugouSignature::Android,
            cookie: KugouCookie::new(),
            encrypt_key: false,
            clear_default_params: false,
            skip_signature: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct KugouResponse {
    pub body: Value,
    pub cookies: Vec<String>,
    pub ssa_code: Option<String>,
}

#[derive(Clone)]
pub struct KugouClient {
    http: Client,
}

impl KugouClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
        }
    }

    pub fn with_http_client(http: Client) -> Self {
        Self { http }
    }

    pub async fn current_cookie(&self) -> KugouCookie {
        auth_session::get_provider_cookie(&ProviderId::Kugou)
            .await
            .map(|value| parse_cookie(&value))
            .unwrap_or_default()
    }

    pub(super) async fn current_auth(&self) -> (KugouCookie, KugouAuth) {
        let cookie = self.current_cookie().await;
        let auth = KugouAuth::from_cookie(&cookie);
        (cookie, auth)
    }

    pub async fn search(&self, keyword: &str, page: u32, page_size: u32) -> ProviderResult<Value> {
        let mut request = KugouRequest::new(Method::GET, "/v3/search/song");
        request.params = KugouParams::from([
            ("albumhide".to_owned(), Value::from(0)),
            ("iscorrection".to_owned(), Value::from(1)),
            ("keyword".to_owned(), Value::String(keyword.to_owned())),
            ("nocollect".to_owned(), Value::from(0)),
            ("page".to_owned(), Value::from(page.max(1))),
            ("pagesize".to_owned(), Value::from(page_size.clamp(1, 100))),
            (
                "platform".to_owned(),
                Value::String("AndroidFilter".to_owned()),
            ),
        ]);
        request
            .headers
            .insert("x-router".to_owned(), "complexsearch.kugou.com".to_owned());
        request.cookie = self.current_cookie().await;
        Ok(self.request(request).await?.body)
    }

    pub async fn song_url(
        &self,
        hash: &str,
        album_id: u64,
        album_audio_id: u64,
        quality: &str,
    ) -> ProviderResult<Value> {
        let mut request = KugouRequest::new(Method::GET, "/v5/url");
        request.params = KugouParams::from([
            ("album_id".to_owned(), Value::from(album_id)),
            ("album_audio_id".to_owned(), Value::from(album_audio_id)),
            ("area_code".to_owned(), Value::from(1)),
            ("behavior".to_owned(), Value::String("play".to_owned())),
            ("cdnBackup".to_owned(), Value::from(1)),
            ("cmd".to_owned(), Value::from(26)),
            ("hash".to_owned(), Value::String(hash.to_ascii_lowercase())),
            ("IsFreePart".to_owned(), Value::from(0)),
            ("module".to_owned(), Value::String(String::new())),
            ("page_id".to_owned(), Value::from(151_369_488)),
            ("pid".to_owned(), Value::from(2)),
            ("pidversion".to_owned(), Value::from(3001)),
            (
                "ppage_id".to_owned(),
                Value::String("463467626,350369493,788954147".to_owned()),
            ),
            ("quality".to_owned(), Value::String(quality.to_owned())),
            (
                "ssa_flag".to_owned(),
                Value::String("is_fromtrack".to_owned()),
            ),
            ("version".to_owned(), Value::from(11430)),
        ]);
        request.encrypt_key = true;
        request
            .headers
            .insert("x-router".to_owned(), "trackercdn.kugou.com".to_owned());
        request.cookie = self.current_cookie().await;
        Ok(self.request(request).await?.body)
    }

    pub async fn song_url_h5(
        &self,
        hash: &str,
        album_id: u64,
        album_audio_id: u64,
        quality: &str,
    ) -> ProviderResult<Value> {
        let (cookie, auth) = self.current_auth().await;
        self.ensure_playback(&auth)?;
        let mut request = self.h5_request(
            Method::GET,
            "/v5/url",
            "trackercdn.kugou.com",
            &auth,
            cookie,
            None::<&Value>,
        )?;
        request.params.extend(KugouParams::from([
            ("album_id".to_owned(), Value::from(album_id)),
            ("album_audio_id".to_owned(), Value::from(album_audio_id)),
            ("area_code".to_owned(), Value::from(1)),
            ("behavior".to_owned(), Value::String("play".to_owned())),
            ("cdnBackup".to_owned(), Value::from(1)),
            ("cmd".to_owned(), Value::from(26)),
            ("hash".to_owned(), Value::String(hash.to_ascii_lowercase())),
            ("IsFreePart".to_owned(), Value::from(0)),
            ("module".to_owned(), Value::String(String::new())),
            ("pid".to_owned(), Value::from(2)),
            ("pidversion".to_owned(), Value::from(3001)),
            ("quality".to_owned(), Value::String(quality.to_owned())),
            (
                "ssa_flag".to_owned(),
                Value::String("is_fromtrack".to_owned()),
            ),
            ("version".to_owned(), Value::from(11430)),
        ]));
        request.params.insert(
            "key".to_owned(),
            Value::String(sign_key(hash, &default_mid(&auth), &auth.user_id, "1014")),
        );
        self.sign_h5_request(&mut request);
        Ok(self.request(request).await?.body)
    }

    pub(super) async fn song_url_mobile(&self, hash: &str, album_id: u64) -> ProviderResult<Value> {
        let (cookie, auth) = self.current_auth().await;
        let mut query = vec![
            ("cmd", "playInfo".to_owned()),
            ("hash", hash.to_owned()),
            ("key", md5_hex(format!("{hash}kgcloud").as_bytes())),
            ("album_id", album_id.to_string()),
            ("pid", "1".to_owned()),
            ("forceDown", "0".to_owned()),
            (
                "vip",
                if auth.playback_ready() { "1" } else { "65530" }.to_owned(),
            ),
        ];
        if !auth.user_id.is_empty() {
            query.push(("userid", auth.user_id));
        }
        if !auth.token.is_empty() {
            query.push(("token", auth.token));
        }
        self.simple_get(
            "http://m.kugou.com/app/i/getSongInfo.php",
            query,
            &cookie,
            Some("https://m.kugou.com/"),
        )
        .await
    }

    pub(super) async fn song_url_web(
        &self,
        hash: &str,
        album_id: u64,
        album_audio_id: u64,
    ) -> ProviderResult<Value> {
        let (cookie, auth) = self.current_auth().await;
        let query = vec![
            ("r", "play/getdata".to_owned()),
            ("hash", hash.to_owned()),
            ("album_id", album_id.to_string()),
            ("album_audio_id", album_audio_id.to_string()),
            ("appid", "1014".to_owned()),
            ("platid", "4".to_owned()),
            ("mid", default_mid(&auth)),
            ("dfid", default_dfid(&auth)),
            ("userid", auth.user_id),
            ("token", auth.token),
        ];
        self.simple_get(
            "https://wwwapi.kugou.com/yy/index.php",
            query,
            &cookie,
            None,
        )
        .await
    }

    pub(super) async fn user_collection_list(&self) -> ProviderResult<KugouCollectionResp> {
        let (cookie, auth) = self.current_auth().await;
        self.ensure_playback(&auth)?;
        let payload = KugouPlaylistListRequest {
            userid: numeric_id(&auth.user_id),
            token: &auth.token,
            total_ver: 979,
            r#type: 2,
            page: 1,
            pagesize: 50,
        };
        let body = self
            .h5_json(
                "/v7/get_all_list",
                "cloudlist.service.kugou.com",
                &auth,
                cookie,
                &payload,
            )
            .await?;
        serde_json::from_value(body).map_err(|error| ProviderError {
            code: ProviderErrorCode::InvalidResponse,
            provider: ProviderId::Kugou,
            message: format!("decode kugou collection response: {error}"),
            retryable: false,
            action: Some("user_collection_list".to_owned()),
            raw_message: None,
        })
    }

    pub(super) async fn playlist_tracks_page(
        &self,
        list_id: u64,
        page: u32,
        page_size: u32,
    ) -> ProviderResult<Value> {
        let (cookie, auth) = self.current_auth().await;
        self.ensure_playback(&auth)?;
        let payload = KugouPlaylistTracksRequest {
            listid: list_id,
            userid: numeric_id(&auth.user_id),
            area_code: 1,
            show_relate_goods: 0,
            pagesize: page_size.clamp(1, 50),
            allplatform: 1,
            show_cover: 1,
            r#type: 0,
            token: &auth.token,
            page: page.max(1),
        };
        self.h5_json(
            "/v4/get_list_all_file",
            "cloudlist.service.kugou.com",
            &auth,
            cookie,
            &payload,
        )
        .await
    }

    pub(super) async fn add_song_to_playlist(
        &self,
        payload: &KugouAddSongRequest<'_>,
    ) -> ProviderResult<Value> {
        let (cookie, auth) = self.current_auth().await;
        self.ensure_playback(&auth)?;
        self.h5_json(
            "/v6/add_song",
            "cloudlist.service.kugou.com",
            &auth,
            cookie,
            payload,
        )
        .await
    }

    pub(super) async fn delete_song_from_playlist(
        &self,
        payload: &KugouDeleteSongRequest<'_>,
    ) -> ProviderResult<Value> {
        let (cookie, auth) = self.current_auth().await;
        self.ensure_playback(&auth)?;
        self.h5_json(
            "/v4/delete_songs",
            "cloudlist.service.kugou.com",
            &auth,
            cookie,
            payload,
        )
        .await
    }

    pub(super) async fn album_detail(&self, id: &str) -> ProviderResult<Value> {
        let body = serde_json::json!({
            "data": [{ "album_id": id }],
            "is_buy": 0,
            "fields": "album_id,album_name,publish_date,sizable_cover,intro,language,is_publish,heat,type,quality,authors,exclusive,author_name,trans_param"
        });
        self.android_json(
            "/kmr/v2/albums",
            "openapi.kugou.com",
            self.current_cookie().await,
            &body,
        )
        .await
    }

    pub(super) async fn album_songs(
        &self,
        id: &str,
        page: u32,
        pagesize: u32,
    ) -> ProviderResult<Value> {
        let body = serde_json::json!({
            "album_id": id,
            "is_buy": "",
            "page": page.max(1),
            "pagesize": pagesize.clamp(1, 50)
        });
        self.android_json(
            "/v1/album_audio/lite",
            "openapi.kugou.com",
            self.current_cookie().await,
            &body,
        )
        .await
    }

    pub(super) async fn lyric_search(&self, hash: &str) -> ProviderResult<KugouLyricSearchResp> {
        let mut request = KugouRequest::new(Method::GET, "/search");
        request.base_url = Some("https://lyrics.kugou.com".to_owned());
        request.clear_default_params = true;
        request.skip_signature = true;
        request.params = KugouParams::from([
            ("client".to_owned(), Value::String("pc".to_owned())),
            ("keyword".to_owned(), Value::String(String::new())),
            ("man".to_owned(), Value::String("no".to_owned())),
            ("hash".to_owned(), Value::String(hash.to_ascii_lowercase())),
            ("timelength".to_owned(), Value::from(0)),
            ("ver".to_owned(), Value::from(1)),
        ]);
        request.cookie = self.current_cookie().await;
        self.request_model(request, "lyric_search").await
    }

    pub(super) async fn lyric(&self, id: u64, access_key: &str) -> ProviderResult<KugouLyricResp> {
        let mut request = KugouRequest::new(Method::GET, "/download");
        request.base_url = Some("https://lyrics.kugou.com".to_owned());
        request.params = KugouParams::from([
            ("accesskey".to_owned(), Value::String(access_key.to_owned())),
            ("charset".to_owned(), Value::String("utf8".to_owned())),
            ("client".to_owned(), Value::String("android".to_owned())),
            ("fmt".to_owned(), Value::String("lrc".to_owned())),
            ("id".to_owned(), Value::from(id)),
            ("ver".to_owned(), Value::from(1)),
        ]);
        request.cookie = self.current_cookie().await;
        self.request_model(request, "lyric").await
    }

    pub(super) async fn lyric_krc(
        &self,
        id: u64,
        access_key: &str,
    ) -> ProviderResult<KugouLyricResp> {
        let mut request = KugouRequest::new(Method::GET, "/download");
        request.base_url = Some("https://lyrics.kugou.com".to_owned());
        request.params = KugouParams::from([
            ("accesskey".to_owned(), Value::String(access_key.to_owned())),
            ("charset".to_owned(), Value::String("utf8".to_owned())),
            ("client".to_owned(), Value::String("android".to_owned())),
            ("fmt".to_owned(), Value::String("krc".to_owned())),
            ("id".to_owned(), Value::from(id)),
            ("ver".to_owned(), Value::from(1)),
        ]);
        request.cookie = self.current_cookie().await;
        self.request_model(request, "lyric_krc").await
    }

    async fn request_model<T: DeserializeOwned>(
        &self,
        request: KugouRequest,
        action: &str,
    ) -> ProviderResult<T> {
        let response = self.request(request).await?;
        let raw = response.body.to_string();
        serde_json::from_value(response.body).map_err(|err| ProviderError {
            code: ProviderErrorCode::InvalidResponse,
            provider: ProviderId::Kugou,
            message: format!("decode kugou {action} response: {err}"),
            retryable: false,
            action: Some(action.to_owned()),
            raw_message: Some(raw),
        })
    }

    fn h5_request<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        router: &str,
        auth: &KugouAuth,
        mut cookie: KugouCookie,
        body: Option<&T>,
    ) -> ProviderResult<KugouRequest> {
        let clienttime = unix_millis().to_string();
        let body = body
            .map(serde_json::to_value)
            .transpose()
            .map_err(|error| unavailable_error(error.to_string()))?;
        let mut request = KugouRequest::new(method, path);
        request.clear_default_params = true;
        request.signature = KugouSignature::H5;
        request.params = KugouParams::from([
            ("appid".to_owned(), Value::from(1014)),
            ("clienttime".to_owned(), Value::String(clienttime.clone())),
            ("clientver".to_owned(), Value::String("20000".to_owned())),
            ("dfid".to_owned(), Value::String(default_dfid(auth))),
            ("mid".to_owned(), Value::String(default_mid(auth))),
            ("srcappid".to_owned(), Value::String("2919".to_owned())),
            ("token".to_owned(), Value::String(auth.token.clone())),
            ("userid".to_owned(), Value::from(numeric_id(&auth.user_id))),
            ("uuid".to_owned(), Value::String(clienttime)),
        ]);
        request.body = body.map(KugouRequestBody::Json);
        cookie
            .entry("kg_mid".to_owned())
            .or_insert_with(|| default_mid(auth));
        cookie
            .entry("kg_dfid".to_owned())
            .or_insert_with(|| default_dfid(auth));
        request.cookie = cookie;
        request
            .headers
            .insert("User-Agent".to_owned(), H5_USER_AGENT.to_owned());
        request
            .headers
            .insert("x-router".to_owned(), router.to_owned());
        Ok(request)
    }

    async fn h5_json<T: Serialize>(
        &self,
        path: &str,
        router: &str,
        auth: &KugouAuth,
        cookie: KugouCookie,
        payload: &T,
    ) -> ProviderResult<Value> {
        let mut request =
            self.h5_request(Method::POST, path, router, auth, cookie, Some(payload))?;
        request.params.insert("plat".to_owned(), Value::from(1));
        self.sign_h5_request(&mut request);
        Ok(self.request(request).await?.body)
    }

    async fn android_json<T: Serialize>(
        &self,
        path: &str,
        router: &str,
        cookie: KugouCookie,
        payload: &T,
    ) -> ProviderResult<Value> {
        let body =
            serde_json::to_value(payload).map_err(|error| unavailable_error(error.to_string()))?;
        let mut request = KugouRequest::new(Method::POST, path);
        request.body = Some(KugouRequestBody::Json(body));
        request.cookie = cookie;
        request
            .headers
            .insert("x-router".to_owned(), router.to_owned());
        request
            .headers
            .insert("kg-tid".to_owned(), "255".to_owned());
        Ok(self.request(request).await?.body)
    }

    fn sign_h5_request(&self, request: &mut KugouRequest) {
        let body = request.body.as_ref().map(KugouRequestBody::bytes);
        request.params.insert(
            "signature".to_owned(),
            Value::String(signature_h5(&request.params, body.as_deref())),
        );
    }

    fn ensure_playback(&self, auth: &KugouAuth) -> ProviderResult<()> {
        auth.playback_ready()
            .then_some(())
            .ok_or_else(|| ProviderError {
                code: ProviderErrorCode::LoginRequired,
                provider: ProviderId::Kugou,
                message: "kugou login with userid and token required".to_owned(),
                retryable: false,
                action: Some("login".to_owned()),
                raw_message: None,
            })
    }

    async fn simple_get(
        &self,
        url: &str,
        query: Vec<(&str, String)>,
        cookie: &KugouCookie,
        referer: Option<&str>,
    ) -> ProviderResult<Value> {
        let mut request = self
            .http
            .get(url)
            .query(&query)
            .header("user-agent", H5_USER_AGENT)
            .header("cookie", cookie_header(cookie));
        if let Some(referer) = referer {
            request = request.header("referer", referer);
        }
        let response = request
            .send()
            .await
            .map_err(|error| unavailable_error(error.to_string()))?;
        parse_response(response).await.map(|response| response.body)
    }

    pub async fn request(&self, request: KugouRequest) -> ProviderResult<KugouResponse> {
        let clienttime = unix_seconds().to_string();
        let auth = KugouAuth::from_cookie(&request.cookie);
        let dfid = request
            .cookie
            .get("dfid")
            .or_else(|| request.cookie.get("kg_dfid"))
            .or_else(|| request.cookie.get("KG_DFID"))
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| default_dfid(&auth));
        let mid = request
            .cookie
            .get("KUGOU_API_MID")
            .or_else(|| request.cookie.get("kg_mid"))
            .or_else(|| request.cookie.get("KG_MID"))
            .cloned()
            .unwrap_or_else(|| default_mid(&auth));

        let mut params = if request.clear_default_params {
            request.params
        } else {
            let mut params = KugouParams::from([
                ("dfid".to_owned(), Value::String(dfid.clone())),
                ("mid".to_owned(), Value::String(mid.clone())),
                ("uuid".to_owned(), Value::String("-".to_owned())),
                ("appid".to_owned(), Value::String(APP_ID.to_owned())),
                (
                    "clientver".to_owned(),
                    Value::String(CLIENT_VERSION.to_owned()),
                ),
                ("clienttime".to_owned(), Value::String(clienttime.clone())),
            ]);
            if let Some(token) = request
                .cookie
                .get("token")
                .or_else(|| request.cookie.get("Token"))
                .filter(|value| !value.is_empty())
                .cloned()
                .or_else(|| (!auth.token.is_empty()).then(|| auth.token.clone()))
            {
                params.insert("token".to_owned(), Value::String(token));
            }
            if let Some(userid) = request
                .cookie
                .get("userid")
                .or_else(|| request.cookie.get("UserId"))
                .filter(|value| *value != "0")
                .cloned()
                .or_else(|| (!auth.user_id.is_empty()).then(|| auth.user_id.clone()))
            {
                params.insert("userid".to_owned(), Value::String(userid));
            }
            params.extend(request.params);
            params
        };

        if request.encrypt_key {
            let hash = params
                .get("hash")
                .map(json_value_to_string)
                .unwrap_or_else(|| "undefined".to_owned());
            let userid = params
                .get("userid")
                .map(json_value_to_string)
                .unwrap_or_else(|| "0".to_owned());
            let appid = params
                .get("appid")
                .map(json_value_to_string)
                .unwrap_or_else(|| APP_ID.to_owned());
            params.insert(
                "key".to_owned(),
                Value::String(sign_key(&hash, &mid, &userid, &appid)),
            );
        }

        let body = request.body.as_ref().map(KugouRequestBody::bytes);
        let body_content_type = request
            .body
            .as_ref()
            .and_then(KugouRequestBody::content_type);
        let has_content_type = request
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("content-type"));
        if !request.skip_signature && !params.contains_key("signature") {
            let signature = match request.signature {
                KugouSignature::Android => signature_android(&params, body.as_deref()),
                KugouSignature::Web => signature_web(&params),
                KugouSignature::H5 => signature_h5(&params, body.as_deref()),
                KugouSignature::Register => signature_register(&params),
            };
            params.insert("signature".to_owned(), Value::String(signature));
        }

        let url = format!(
            "{}/{}",
            request
                .base_url
                .as_deref()
                .unwrap_or(GATEWAY_URL)
                .trim_end_matches('/'),
            request.path.trim_start_matches('/')
        );
        let query = params
            .iter()
            .map(|(key, value)| (key.as_str(), json_value_to_string(value)))
            .collect::<Vec<_>>();
        let mut request_builder = self.http.request(request.method, url).query(&query).header(
            "user-agent",
            request
                .headers
                .get("User-Agent")
                .cloned()
                .unwrap_or_else(|| USER_AGENT.to_owned()),
        );
        for (name, value) in request.headers {
            if !name.eq_ignore_ascii_case("user-agent") {
                request_builder = request_builder.header(name, value);
            }
        }
        if let Some(content_type) = body_content_type.filter(|_| !has_content_type) {
            request_builder = request_builder.header("content-type", content_type);
        }
        request_builder = request_builder
            .header("dfid", dfid)
            .header("clienttime", clienttime)
            .header("mid", mid)
            .header("kg-rc", "1")
            .header("kg-thash", "5d816a0")
            .header("kg-rec", "1")
            .header("kg-rf", "B9EDA08A64250DEFFBCADDEE00F8F25F");
        if !request.cookie.is_empty() {
            request_builder = request_builder.header("cookie", cookie_header(&request.cookie));
        }
        if let Some(body) = body {
            request_builder = request_builder.body(body);
        }

        let response = request_builder
            .send()
            .await
            .map_err(|error| unavailable_error(error.to_string()))?;
        parse_response(response).await
    }
}

impl Default for KugouClient {
    fn default() -> Self {
        Self::new()
    }
}

pub fn signature_web(params: &KugouParams) -> String {
    let params = signature_pairs(params).join("");
    md5_hex(format!("{WEB_SIGNATURE_SALT}{params}{WEB_SIGNATURE_SALT}").as_bytes())
}

pub fn signature_h5(params: &KugouParams, body: Option<&[u8]>) -> String {
    let mut params = signature_pairs(params);
    if let Some(body) = body {
        params.push(String::from_utf8_lossy(body).into_owned());
    }
    md5_hex(
        format!(
            "{WEB_SIGNATURE_SALT}{}{WEB_SIGNATURE_SALT}",
            params.join("")
        )
        .as_bytes(),
    )
}

pub fn signature_android(params: &KugouParams, body: Option<&[u8]>) -> String {
    let params = signature_pairs(params).join("");
    let mut hasher = Md5::new();
    hasher.update(ANDROID_SIGNATURE_SALT.as_bytes());
    hasher.update(params.as_bytes());
    if let Some(body) = body {
        hasher.update(body);
    }
    hasher.update(ANDROID_SIGNATURE_SALT.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn signature_register(params: &KugouParams) -> String {
    let mut values = params
        .values()
        .map(json_value_to_string)
        .collect::<Vec<_>>();
    values.sort();
    md5_hex(
        format!(
            "{REGISTER_SIGNATURE_SALT}{}{REGISTER_SIGNATURE_SALT}",
            values.join("")
        )
        .as_bytes(),
    )
}

pub fn sign_key(hash: &str, mid: &str, userid: &str, appid: &str) -> String {
    md5_hex(format!("{hash}{SIGN_KEY_SALT}{appid}{mid}{userid}").as_bytes())
}

async fn parse_response(response: Response) -> ProviderResult<KugouResponse> {
    let http_status = response.status();
    let cookies = response_cookies(&response);
    let ssa_code = response
        .headers()
        .get("ssa-code")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let text = response
        .text()
        .await
        .map_err(|error| unavailable_error(error.to_string()))?;
    let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
    let api_failed = body.get("status").and_then(Value::as_i64) == Some(0)
        || body
            .get("error_code")
            .and_then(Value::as_i64)
            .is_some_and(|code| code != 0);
    if !http_status.is_success() || api_failed {
        return Err(ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: ProviderId::Kugou,
            message: body
                .get("msg")
                .or_else(|| body.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("kugou upstream error")
                .to_owned(),
            retryable: true,
            action: None,
            raw_message: Some(body.to_string()),
        });
    }
    Ok(KugouResponse {
        body,
        cookies,
        ssa_code,
    })
}

fn signature_pairs(params: &KugouParams) -> Vec<String> {
    params
        .iter()
        .map(|(key, value)| format!("{key}={}", json_value_to_string(value)))
        .collect()
}

fn json_value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn response_cookies(response: &Response) -> Vec<String> {
    response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|header| header.to_str().ok())
        .filter_map(|header| header.split(';').next())
        .map(str::trim)
        .filter(|cookie| !cookie.is_empty() && cookie.contains('='))
        .map(ToOwned::to_owned)
        .collect()
}

fn cookie_header(cookie: &KugouCookie) -> String {
    cookie
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn parse_cookie(cookie: &str) -> KugouCookie {
    cookie
        .split(';')
        .filter_map(|part| {
            let (key, value) = part.trim().split_once('=')?;
            let key = key.trim();
            (!key.is_empty()).then(|| (key.to_owned(), value.trim().to_owned()))
        })
        .collect()
}

fn md5_hex(value: &[u8]) -> String {
    format!("{:x}", Md5::digest(value))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn numeric_id(value: &str) -> u64 {
    value.parse().unwrap_or_default()
}

fn default_mid(auth: &KugouAuth) -> String {
    if !auth.mid.is_empty() {
        auth.mid.clone()
    } else {
        md5_hex(format!("mineradio:{}", auth.user_id).as_bytes())
    }
}

fn default_dfid(auth: &KugouAuth) -> String {
    if !auth.dfid.is_empty() {
        auth.dfid.clone()
    } else {
        "-".to_owned()
    }
}

fn unavailable_error(message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: ProviderId::Kugou,
        message: message.clone(),
        retryable: true,
        action: None,
        raw_message: Some(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_sort_parameters_like_the_js_client() {
        let params = KugouParams::from([
            ("b".to_owned(), Value::String("2".to_owned())),
            ("a".to_owned(), Value::String("1".to_owned())),
        ]);

        assert_eq!(signature_web(&params), "70ccbef64fdcc9271fe883d1d7f07395");
        assert_eq!(
            signature_android(&params, Some(br#"{"name":"test"}"#)),
            "f3e569d8863a00ed4bac93d6897dba77"
        );
        assert_eq!(
            signature_register(&params),
            "3be0f2ebde7da28161927749ab76ba88"
        );
    }

    #[test]
    fn json_request_body_declares_json_content_type() {
        assert_eq!(
            KugouRequestBody::Json(serde_json::json!({})).content_type(),
            Some("application/json")
        );
        assert_eq!(KugouRequestBody::Text(String::new()).content_type(), None);
        assert_eq!(KugouRequestBody::Bytes(Vec::new()).content_type(), None);
    }

    #[test]
    fn sign_key_matches_the_android_algorithm() {
        assert_eq!(
            sign_key("hash", "mid", "42", "1005"),
            "d467a74e2b00b07c297161131cfd5db4"
        );
    }
}
