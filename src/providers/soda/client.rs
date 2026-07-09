use anyhow::Context;
use reqwest::{
    Client,
    header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};

use crate::providers::{
    Result,
    error::{ProviderError, ProviderErrorCode},
};
use crate::services::auth_session;

const SEARCH_URL: &str = "https://api.qishui.com/luna/pc/search/track?q=&aid=386088&app_name=&region=&geo_region=&os_region=&sim_region=&device_id=&cdid=&iid=&version_name=&version_code=&channel=&build_mode=&network_carrier=&ac=&tz_name=&resolution=&device_platform=&device_type=&os_version=&fp=&cursor=&search_id=&search_method=input&debug_params=&from_search_id=&search_scene=";
const TRACK_URL: &str =
    "https://api.qishui.com/luna/pc/track_v2?track_id=&media_type=track&queue_type=&aid=386088&iid=27960026095955";
const PLAYLIST_LIST_URL: &str = "https://api.qishui.com/luna/pc/me/playlist?aid=386088";
const PLAYLIST_DETAIL_URL: &str = "https://api.qishui.com/luna/pc/playlist/detail?aid=386088";
const ME_URL: &str = "https://api.qishui.com/luna/pc/me?aid=386088&version_code=30050100";
const COLLECTION_MEDIA_URL: &str = "https://api.qishui.com/luna/pc/me/collection/media?aid=386088";
const COLLECTION_MEDIA_DELETE_URL: &str =
    "https://api.qishui.com/luna/pc/me/collection/media/delete?aid=386088";
const LOGOUT_URL: &str = "https://api.qishui.com/passport/web/logout/?need_redirect=0&iid=27960026095955&device_platform=PC&version_code=3.5.1&aid=386088";

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

    pub async fn search(&self, keyword: &str) -> Result<Value> {
        let mut url = reqwest::Url::parse(SEARCH_URL).map_err(internal_error)?;
        url.query_pairs_mut().append_pair("q", keyword);
        self.get_json(url.to_string(), self.current_cookie().await.as_deref())
            .await
    }

    pub async fn track_detail(&self, track_id: &str) -> Result<Value> {
        let mut url = reqwest::Url::parse(TRACK_URL).map_err(internal_error)?;
        url.query_pairs_mut().append_pair("track_id", track_id);
        self.get_json(url.to_string(), self.current_cookie().await.as_deref())
            .await
    }

    pub async fn playlist_list(&self) -> Result<Value> {
        self.get_json(PLAYLIST_LIST_URL.to_owned(), self.current_cookie().await.as_deref())
            .await
    }

    pub async fn playlist_detail(&self, playlist_id: &str) -> Result<Value> {
        let cookie = self.current_cookie().await;
        let mut first = self
            .get_json(
                playlist_detail_url(playlist_id, "1", 20)?,
                cookie.as_deref(),
            )
            .await?;
        let mut merged = first
            .get("media_resources")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut cursor = next_cursor(&first);

        while !cursor.is_empty() {
            let page = self
                .get_json(
                    playlist_detail_url(playlist_id, &cursor, 20)?,
                    cookie.as_deref(),
                )
                .await?;
            if let Some(items) = page.get("media_resources").and_then(Value::as_array) {
                merged.extend(items.iter().cloned());
            }
            cursor = next_cursor(&page);
        }

        if let Some(root) = first.as_object_mut() {
            root.insert("media_resources".to_owned(), Value::Array(merged));
        }
        Ok(first)
    }

    pub async fn login_status(&self) -> Result<Value> {
        self.get_json(ME_URL.to_owned(), self.current_cookie().await.as_deref())
            .await
    }

    pub async fn collection_media(&self, track_id: &str, liked: bool) -> Result<(Value, u16)> {
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

    pub async fn logout(&self) -> Result<Value> {
        self.get_json(LOGOUT_URL.to_owned(), self.current_cookie().await.as_deref())
            .await
    }

    pub async fn read_json_url(&self, url: &str) -> Result<Value> {
        self.get_json(url.to_owned(), self.current_cookie().await.as_deref())
            .await
    }

    async fn get_json(&self, url: String, cookie: Option<&str>) -> Result<Value> {
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
}

fn next_cursor(body: &Value) -> String {
    if body.get("has_more").and_then(Value::as_bool) != Some(true) {
        return String::new();
    }
    body.get("next_cursor")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn playlist_detail_url(playlist_id: &str, cursor: &str, count: u32) -> Result<String> {
    let mut url = reqwest::Url::parse(PLAYLIST_DETAIL_URL).map_err(internal_error)?;
    url.query_pairs_mut()
        .append_pair("playlist_id", playlist_id)
        .append_pair("cursor", cursor)
        .append_pair("count", &count.to_string());
    Ok(url.to_string())
}

fn header_value(value: &str) -> Result<HeaderValue> {
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
