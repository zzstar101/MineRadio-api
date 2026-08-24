use anyhow::Context;
use reqwest::{
    Client,
    header::{COOKIE, HeaderMap, HeaderValue},
};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::{Value, json};

use crate::providers::{
    ProviderId, ProviderResult,
    error::{ProviderError, ProviderErrorCode},
    soda::model::{
        SodaCollectionResp, SodaLoginStatusResp, SodaPlaylistDetailResp, SodaPlaylistListResp,
        SodaSearch2Resp, SodaTrackV2Resp,
    },
};
use crate::utils::cryptors::q9v_with_body;
use crate::{auth_session, utils::cryptors::qq::x5};

use super::model::{
    SodaAlbumDetailResp, SodaCollectionListResp, SodaMultiSearchResp, SodaSongUrlResp,
};

const SEARCH_URL: &str = "https://api.qishui.com/luna/pc/search/track?aid=386088&app_name=luna_pc&region=cn&geo_region=cn&os_region=cn&sim_region=&device_id=3753066532709850&cdid=&iid=357778617272924&version_name=3.6.0&version_code=30060000&channel=official&build_mode=master&network_carrier=&ac=wifi&resolution=&fp=3753066532709850&search_method=input&debug_params=&from_search_id=&search_scene=";
const SEARCH_FALLBACK_URL: &str = "https://api-vehicle.volcengine.com/v2/search/type";
const SEARCH_ALBUM_URL: &str = "https://api.qishui.com/luna/pc/search/album?aid=386088";
const SEARCH_PLAYLIST_URL: &str = "https://api.qishui.com/luna/pc/search/playlist?aid=386088";
const TRACK_URL: &str = "https://api.qishui.com/luna/pc/track_v2?aid=386088&app_name=luna_pc&region=cn&geo_region=cn&os_region=cn&sim_region=&device_id=3753066532709850&cdid=&iid=357778617272924&version_name=3.7.0&version_code=30070000&channel=official";
const PLAYLIST_LIST_URL: &str = "https://api.qishui.com/luna/pc/me/playlist?aid=386088";
const PLAYLIST_DETAIL_URL: &str = "https://api.qishui.com/luna/pc/playlist/detail?aid=386088";
const ME_URL: &str = "https://api.qishui.com/luna/pc/me?aid=386088&version_code=30050100";
const COLLECTION_MEDIA_APPEND_URL: &str =
    "https://api.qishui.com/luna/pc/me/collection/media?aid=386088";
const COLLECTION_MEDIA_DELETE_URL: &str =
    "https://api.qishui.com/luna/pc/me/collection/media/delete?aid=386088";
const PLAYLIST_MEDIA_URL: &str = "https://api.qishui.com/luna/pc/me/playlist/media/append?aid=386088&iid=357778617272924&version_name=3.6.0";
const PLAYLIST_MEDIA_DELETE_URL: &str = "https://api.qishui.com/luna/pc/me/playlist/media/delete?aid=386088&iid=357778617272924&version_name=3.6.0";
const LOGOUT_URL: &str = "https://api.qishui.com/passport/web/logout/?need_redirect=0&iid=27960026095955&device_platform=PC&version_code=3.5.1&aid=386088";
const COLLECTION_LIST_URL: &str = "https://api.qishui.com/luna/pc/me/collection/mixed?aid=386088&app_name=luna_pc&iid=3242894632956240&version_name=3.5.2&version_code=30050200&channel=official&item_types=album&item_types=playlist";
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
        auth_session::get_provider_cookie(&ProviderId::Soda).await
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
                provider: ProviderId::Soda,
                message: "soda login required".to_owned(),
                retryable: true,
                action: Some("login".to_owned()),
                raw_message: None,
            });
        }
        Ok(())
    }

    pub(super) async fn search_track(
        &self,
        keyword: &str,
        cursor: u32,
    ) -> ProviderResult<SodaMultiSearchResp> {
        let mut url = reqwest::Url::parse(SEARCH_URL).map_err(internal_error)?;
        url.query_pairs_mut()
            .append_pair("q", keyword)
            .append_pair("cursor", &cursor.to_string())
            .append_pair("search_id", &x5());
        self.get_model(&url.to_string(), "search").await
    }

    pub(super) async fn search_track_fallback(
        &self,
        keyword: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<SodaSearch2Resp> {
        let mut url = reqwest::Url::parse(SEARCH_FALLBACK_URL).map_err(internal_error)?;
        url.query_pairs_mut()
            .append_pair("keyword", keyword)
            .append_pair("search_source", "qishui")
            .append_pair("search_type", "music")
            .append_pair("real_offset", &offset.to_string())
            .append_pair("limit", &limit.to_string());
        self.get_model(&url.to_string(), "search_fallback").await
    }

    pub(super) async fn search_album(
        &self,
        keyword: &str,
        cursor: u32,
    ) -> ProviderResult<SodaMultiSearchResp> {
        let mut url = reqwest::Url::parse(SEARCH_ALBUM_URL).map_err(internal_error)?;
        url.query_pairs_mut()
            .append_pair("q", keyword)
            .append_pair("cursor", &cursor.to_string());
        self.get_model(&url.to_string(), "search_album").await
    }

    pub(super) async fn search_playlist(
        &self,
        keyword: &str,
        cursor: u32,
    ) -> ProviderResult<SodaMultiSearchResp> {
        let mut url = reqwest::Url::parse(SEARCH_PLAYLIST_URL).map_err(internal_error)?;
        url.query_pairs_mut()
            .append_pair("q", keyword)
            .append_pair("cursor", &cursor.to_string());
        self.get_model(&url.to_string(), "search_playlist").await
    }

    pub(super) async fn song_url(&self, track_id: &str) -> ProviderResult<SodaSongUrlResp> {
        let info_url = self.track_detail(track_id).await?.get_songurl();
        if info_url.is_empty() {
            return Err(unavailable_error(format!(
                "soda track {track_id} missing url_player_info"
            )));
        }
        self.get_model(&info_url, "song_url").await
    }

    pub(super) async fn lyric(&self, track_id: &str) -> ProviderResult<SodaTrackV2Resp> {
        self.track_detail(track_id).await
    }

    pub(super) async fn track_detail(&self, track_id: &str) -> ProviderResult<SodaTrackV2Resp> {
        self.post_model(
            TRACK_URL,
            json!({
                "track_id": track_id,
                "media_type": "track",
                "queue_type": "favorite_track_playlist",
                "scene_name": "library"
            }),
            "track_detail",
        )
        .await
    }

    pub(super) async fn user_playlist_list(&self) -> ProviderResult<SodaPlaylistListResp> {
        self.get_model(PLAYLIST_LIST_URL, "playlist_list").await
    }
    //这个是收藏的专辑和歌单接口
    pub(super) async fn user_collected_list(&self) -> ProviderResult<SodaCollectionListResp> {
        self.get_model(COLLECTION_LIST_URL, "album_list").await
    }

    pub(super) async fn playlist_detail(
        &self,
        playlist_id: &str,
        offset: u32,
        limit: u32,
    ) -> ProviderResult<SodaPlaylistDetailResp> {
        let mut url = reqwest::Url::parse(PLAYLIST_DETAIL_URL).map_err(internal_error)?;
        url.query_pairs_mut()
            .append_pair("playlist_id", playlist_id)
            .append_pair("cursor", &offset.to_string())
            .append_pair("count", &limit.to_string());
        self.get_model(&url.to_string(), "playlist_detail").await
    }

    pub(super) async fn album_detail(&self, id: &str) -> ProviderResult<SodaAlbumDetailResp> {
        self.get_model(&ALBUM_DETAIL_URL.replace("AID", id), "album_detail")
            .await
    }

    pub(super) async fn login_status(&self) -> ProviderResult<SodaLoginStatusResp> {
        self.get_model(ME_URL, "login_status").await
    }

    pub(super) async fn like_song(
        &self,
        track_id: &str,
        liked: bool,
    ) -> ProviderResult<SodaCollectionResp> {
        let url = if liked {
            COLLECTION_MEDIA_APPEND_URL
        } else {
            COLLECTION_MEDIA_DELETE_URL
        };
        self.post_model(
            url,
            json!({
                "media": [{"type": "track", "id": track_id}],
                "scene": ""
            }),
            "like_song",
        )
        .await
    }

    pub(super) async fn update_song_in_playlist(
        &self,
        playlist_id: &str,
        track_id: &str,
        adding: bool,
    ) -> ProviderResult<IgnoredAny> {
        let url = if adding {
            PLAYLIST_MEDIA_URL
        } else {
            PLAYLIST_MEDIA_DELETE_URL
        };
        self.post_model(
            url,
            json!({
                "playlist_id": playlist_id,
                "media": [{"id": track_id, "type": "track"}]
            }),
            "update_song_in_playlist",
        )
        .await
    }

    pub async fn logout(&self) -> ProviderResult<()> {
        let mut headers = HeaderMap::new();
        if let Some(cookie) = self
            .current_cookie()
            .await
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            headers.insert(COOKIE, header_value(cookie)?);
        }
        self.http
            .get(LOGOUT_URL)
            .headers(headers)
            .send()
            .await
            .context("send soda upstream request")
            .map_err(unavailable_error)?;
        Ok(())
    }

    async fn get_model<T: DeserializeOwned>(&self, url: &str, action: &str) -> ProviderResult<T> {
        let mut headers = HeaderMap::new();
        if let Some(cookie) = self
            .current_cookie()
            .await
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            headers.insert(COOKIE, header_value(cookie)?);
        }
        q9v_with_body(&url, &[], &mut headers);
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
                provider: ProviderId::Soda,
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
            provider: ProviderId::Soda,
            message: format!("decode soda {action} response: {err}"),
            retryable: false,
            action: Some(action.to_owned()),
            raw_message: Some(String::from_utf8_lossy(&body).into_owned()),
        })
    }

    async fn post_model<T: DeserializeOwned>(
        &self,
        url: &str,
        payload: Value,
        action: &str,
    ) -> ProviderResult<T> {
        let mut headers = HeaderMap::new();
        if let Some(cookie) = self
            .current_cookie()
            .await
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            headers.insert(COOKIE, header_value(cookie)?);
        }
        q9v_with_body(&url, (&payload).to_string().as_bytes(), &mut headers);
        let response = self
            .http
            .post(url)
            .headers(headers)
            .json(&payload)
            .send()
            .await
            .context(format!("send soda {action} request"))
            .map_err(unavailable_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ProviderError {
                code: ProviderErrorCode::Unavailable,
                provider: ProviderId::Soda,
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
            provider: ProviderId::Soda,
            message: format!("decode soda {action} response: {err}"),
            retryable: false,
            action: Some(action.to_owned()),
            raw_message: Some(String::from_utf8_lossy(&body).into_owned()),
        })
    }
}

fn header_value(value: &str) -> ProviderResult<HeaderValue> {
    HeaderValue::from_str(value).map_err(internal_error)
}

fn internal_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Internal,
        provider: ProviderId::Soda,
        message: err.to_string(),
        retryable: false,
        action: None,
        raw_message: None,
    }
}

fn unavailable_error(err: impl std::fmt::Display) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: ProviderId::Soda,
        message: err.to_string(),
        retryable: true,
        action: None,
        raw_message: None,
    }
}
