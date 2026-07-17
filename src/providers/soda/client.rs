use anyhow::Context;
use reqwest::{
    Client,
    header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::providers::{
    ProviderResult,
    error::{ProviderError, ProviderErrorCode},
    soda::model::{SodaPLaylistDetailResp, SodaPlaylistListResp, SodaTrackV2Resp},
};
use crate::services::auth_session;

use super::model::{SodaAlbumDetailResp, SodaAlbumListResp, SodaSearchResp, SodaSongUrlResp};

const SEARCH_URL: &str = "https://api.qishui.com/luna/pc/search/track?aid=386088&app_name=&region=&geo_region=&os_region=&sim_region=&device_id=&cdid=&iid=&version_name=&version_code=&channel=&build_mode=&network_carrier=&ac=&tz_name=&resolution=&device_platform=&device_type=&os_version=&fp=&cursor=&search_id=&search_method=input&debug_params=&from_search_id=&search_scene=";
const TRACK_URL: &str = "https://api.qishui.com/luna/pc/track_v2?&media_type=track&queue_type=&aid=386088&iid=27960026095955";
const PLAYLIST_LIST_URL: &str = "https://api.qishui.com/luna/pc/me/playlist?aid=386088";
const PLAYLIST_DETAIL_URL: &str = "https://api.qishui.com/luna/pc/playlist/detail?aid=386088";
const ME_URL: &str = "https://api.qishui.com/luna/pc/me?aid=386088&version_code=30050100";
const COLLECTION_MEDIA_URL: &str = "https://api.qishui.com/luna/pc/me/collection/media?aid=386088";
const COLLECTION_MEDIA_DELETE_URL: &str =
    "https://api.qishui.com/luna/pc/me/collection/media/delete?aid=386088";
const LOGOUT_URL: &str = "https://api.qishui.com/passport/web/logout/?need_redirect=0&iid=27960026095955&device_platform=PC&version_code=3.5.1&aid=386088";
const ALBUM_LIST_URL: &str = "https://api.qishui.com/luna/pc/me/collection/mixed?aid=386088&app_name=luna_pc&iid=3242894632956240&version_name=3.5.2&version_code=30050200&channel=official&item_types=album&item_types=playlist";
const ALBUM_DETAIL_URL: &str = "https://api.qishui.com/luna/pc/albums/AID?aid=386088&app_name=luna_pc&iid=3242894632956240&version_code=30050200&ignore_tracks=false";

#[derive(Clone, Default)]
pub struct SodaClient {
    http: Client,
}

impl SodaClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
        }
    }

    pub async fn current_cookie(&self) -> Option<String> {
        auth_session::get_provider_cookie("soda").await
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
                provider: "soda".to_owned(),
                message: "soda login required".to_owned(),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: None,
            });
        }
        Ok(())
    }

    pub(super) async fn search(&self, keyword: &str) -> ProviderResult<SodaSearchResp> {
        let mut url = reqwest::Url::parse(SEARCH_URL).map_err(internal_error)?;
        url.query_pairs_mut().append_pair("q", keyword);
        self.get_model(
            url.to_string(),
            self.current_cookie().await.as_deref(),
            "search",
        )
        .await
    }

    pub(super) async fn song_url(&self, track_id: &str) -> ProviderResult<SodaSongUrlResp> {
        let info_url = self.track_detail(track_id).await?.get_songurl();
        if info_url.is_empty() {
            return Err(unavailable_error(format!(
                "soda track {track_id} missing url_player_info"
            )));
        }
        self.get_model(
            info_url.to_owned(),
            self.current_cookie().await.as_deref(),
            "song_url",
        )
        .await
    }

    pub(super) async fn lyric(&self, track_id: &str) -> ProviderResult<SodaTrackV2Resp> {
        self.track_detail(track_id).await
    }

    pub(super) async fn track_detail(&self, track_id: &str) -> ProviderResult<SodaTrackV2Resp> {
        let mut url = reqwest::Url::parse(TRACK_URL).map_err(internal_error)?;
        url.query_pairs_mut().append_pair("track_id", track_id);
        self.get_model(
            url.to_string(),
            self.current_cookie().await.as_deref(),
            "track_detail",
        )
        .await
    }

    pub(super) async fn playlist_list(&self) -> ProviderResult<SodaPlaylistListResp> {
        self.get_model(
            PLAYLIST_LIST_URL.to_owned(),
            self.current_cookie().await.as_deref(),
            "playlist_list",
        )
        .await
    }

    pub(super) async fn playlist_detail(
        &self,
        playlist_id: &str,
    ) -> ProviderResult<SodaPLaylistDetailResp> {
        let cookie = self.current_cookie().await;
        self.get_model(
            playlist_detail_url(playlist_id, 0, 20)?,
            cookie.as_deref(),
            "playlist_detail",
        )
        .await
    }

    pub(super) async fn album_list(&self) -> ProviderResult<SodaAlbumListResp> {
        self.get_model(
            ALBUM_LIST_URL.to_owned(),
            self.current_cookie().await.as_deref(),
            "album_list",
        )
        .await
    }

    pub(super) async fn album_detail(&self, id: &str) -> ProviderResult<SodaAlbumDetailResp> {
        self.get_model(
            ALBUM_DETAIL_URL.replace("AID", id),
            self.current_cookie().await.as_deref(),
            "album_detail",
        )
        .await
    }

    pub async fn login_status(&self) -> ProviderResult<Value> {
        self.get_json(ME_URL.to_owned(), self.current_cookie().await.as_deref())
            .await
    }

    pub async fn collection_media(
        &self,
        track_id: &str,
        liked: bool,
    ) -> ProviderResult<(Value, u16)> {
        let url = if liked {
            COLLECTION_MEDIA_URL
        } else {
            COLLECTION_MEDIA_DELETE_URL
        };
        let cookie = self.current_cookie().await;
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(cookie) = cookie.as_deref().filter(|value| !value.trim().is_empty()) {
            headers.insert(COOKIE, header_value(cookie)?);
        }

        let response = self
            .http
            .post(url)
            .headers(headers)
            .body(
                json!({
                    "media": [{"type": "track", "id": track_id}],
                    "scene": ""
                })
                .to_string(),
            )
            .send()
            .await
            .context("send soda collection-media request")
            .map_err(unavailable_error)?;
        let status = response.status().as_u16();
        let body = response
            .json::<Value>()
            .await
            .context("parse soda collection-media response")
            .map_err(unavailable_error)?;
        Ok((body, status))
    }

    pub async fn logout(&self) -> ProviderResult<Value> {
        self.get_json(
            LOGOUT_URL.to_owned(),
            self.current_cookie().await.as_deref(),
        )
        .await
    }

    async fn get_json(&self, url: String, cookie: Option<&str>) -> ProviderResult<Value> {
        let mut headers = HeaderMap::new();
        if let Some(cookie) = cookie.filter(|value| !value.trim().is_empty()) {
            headers.insert(COOKIE, header_value(cookie)?);
        }
        let response = self
            .http
            .get(url)
            .headers(headers)
            .send()
            .await
            .context("send soda upstream request")
            .map_err(unavailable_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ProviderError {
                code: ProviderErrorCode::Unavailable,
                provider: "soda".to_owned(),
                message: format!("soda upstream http {}", status.as_u16()),
                retryable: false,
                action: None,
                raw_message: None,
            });
        }
        response
            .json::<Value>()
            .await
            .context("parse soda upstream response")
            .map_err(unavailable_error)
    }

    async fn get_model<T: DeserializeOwned>(
        &self,
        url: String,
        cookie: Option<&str>,
        action: &str,
    ) -> ProviderResult<T> {
        let mut headers = HeaderMap::new();
        if let Some(cookie) = cookie.filter(|value| !value.trim().is_empty()) {
            headers.insert(COOKIE, header_value(cookie)?);
        }
        let response = self
            .http
            .get(url)
            .headers(headers)
            .send()
            .await
            .context("send soda upstream request")
            .map_err(unavailable_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ProviderError {
                code: ProviderErrorCode::Unavailable,
                provider: "soda".to_owned(),
                message: format!("soda upstream http {}", status.as_u16()),
                retryable: false,
                action: None,
                raw_message: None,
            });
        }
        let body = response
            .bytes()
            .await
            .context("read soda upstream response")
            .map_err(unavailable_error)?;
        serde_json::from_slice(&body).map_err(|err| ProviderError {
            code: ProviderErrorCode::InvalidResponse,
            provider: "soda".to_owned(),
            message: format!("decode soda {action} response: {err}"),
            retryable: false,
            action: Some(action.to_owned()),
            raw_message: Some(String::from_utf8_lossy(&body).into_owned()),
        })
    }
}

fn playlist_detail_url(playlist_id: &str, cursor: u32, count: u32) -> ProviderResult<String> {
    let mut url = reqwest::Url::parse(PLAYLIST_DETAIL_URL).map_err(internal_error)?;
    url.query_pairs_mut()
        .append_pair("playlist_id", playlist_id)
        .append_pair("cursor", &cursor.to_string())
        .append_pair("count", &count.to_string());
    Ok(url.to_string())
}

fn header_value(value: &str) -> ProviderResult<HeaderValue> {
    HeaderValue::from_str(value).map_err(internal_error)
}

fn internal_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Internal,
        provider: "soda".to_owned(),
        message: err.to_string(),
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn unavailable_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: "soda".to_owned(),
        message: err.to_string(),
        retryable: true,
        action: None,
        raw_message: None,
    }
}
