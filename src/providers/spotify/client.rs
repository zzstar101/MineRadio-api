use std::{env, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, Method, StatusCode};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    providers::{
        ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    services::auth_session,
    types::ProviderId,
};

const API_BASE: &str = "https://api.spotify.com/v1";
const ACCOUNTS_BASE: &str = "https://accounts.spotify.com";

#[derive(Clone, Debug)]
struct ClientToken {
    value: String,
    expires_at: std::time::Instant,
}

#[derive(Clone)]
pub struct SpotifyClient {
    http: Client,
    client_id: String,
    client_secret: String,
    market: String,
    client_token: Arc<Mutex<Option<ClientToken>>>,
}

impl SpotifyClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            client_id: env_value("MINERADIO_SPOTIFY_CLIENT_ID")
                .or_else(|| env_value("SPOTIFY_CLIENT_ID"))
                .unwrap_or_default(),
            client_secret: env_value("MINERADIO_SPOTIFY_CLIENT_SECRET")
                .or_else(|| env_value("SPOTIFY_CLIENT_SECRET"))
                .unwrap_or_default(),
            market: env_value("MINERADIO_SPOTIFY_MARKET")
                .or_else(|| env_value("SPOTIFY_MARKET"))
                .unwrap_or_else(|| "US".to_owned()),
            client_token: Arc::new(Mutex::new(None)),
        }
    }

    pub fn market(&self) -> &str {
        &self.market
    }

    pub async fn get(
        &self,
        path: &str,
        query: &[(&str, String)],
        user_only: bool,
    ) -> ProviderResult<Value> {
        self.request(Method::GET, path, query, None, user_only)
            .await
    }

    pub async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
        user_only: bool,
    ) -> ProviderResult<Value> {
        let token = if user_only {
            self.user_access_token().await?
        } else {
            match self.user_access_token().await {
                Ok(token) => token,
                Err(_) => self.client_credentials_token().await?,
            }
        };
        self.request_with_token(method, path, query, body, &token)
            .await
    }

    pub async fn logout(&self) {
        auth_session::clear_runtime_provider_cookie(&ProviderId::Spotify).await;
    }

    async fn request_with_token(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
        token: &str,
    ) -> ProviderResult<Value> {
        let url = format!("{API_BASE}/{}", path.trim_start_matches('/'));
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(token)
            .header("accept", "application/json")
            .query(query);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.map_err(unavailable)?;
        let status = response.status();
        let text = response.text().await.map_err(unavailable)?;
        if !status.is_success() {
            return Err(spotify_error(status, text));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|error| ProviderError {
            code: ProviderErrorCode::InvalidResponse,
            provider: ProviderId::Spotify,
            message: format!("decode spotify response: {error}"),
            retryable: false,
            action: None,
            raw_message: Some(text),
        })
    }

    async fn user_access_token(&self) -> ProviderResult<String> {
        let token = auth_session::get_provider_cookie(&ProviderId::Spotify)
            .await
            .or_else(|| env_value("MINERADIO_SPOTIFY_ACCESS_TOKEN"))
            .or_else(|| env_value("SPOTIFY_ACCESS_TOKEN"));
        token
            .and_then(|value| extract_access_token(&value))
            .ok_or_else(login_required)
    }

    async fn client_credentials_token(&self) -> ProviderResult<String> {
        if self.client_id.is_empty() || self.client_secret.is_empty() {
            return Err(login_required());
        }
        if let Some(cached) = self
            .client_token
            .lock()
            .await
            .as_ref()
            .filter(|token| {
                token.expires_at > std::time::Instant::now() + std::time::Duration::from_secs(30)
            })
            .cloned()
        {
            return Ok(cached.value);
        }
        let basic = STANDARD.encode(format!("{}:{}", self.client_id, self.client_secret));
        let response = self
            .http
            .post(format!("{ACCOUNTS_BASE}/api/token"))
            .header("authorization", format!("Basic {basic}"))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await
            .map_err(unavailable)?;
        let status = response.status();
        let text = response.text().await.map_err(unavailable)?;
        if !status.is_success() {
            return Err(spotify_error(status, text));
        }
        let body: Value = serde_json::from_str(&text)
            .map_err(|error| invalid_response(error.to_string(), text.clone()))?;
        let value = body
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid_response("missing access_token".to_owned(), text.clone()))?
            .to_owned();
        let expires = body
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(3600)
            .max(60);
        *self.client_token.lock().await = Some(ClientToken {
            value: value.clone(),
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(expires),
        });
        Ok(value)
    }
}

impl Default for SpotifyClient {
    fn default() -> Self {
        Self::new()
    }
}

fn env_value(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn extract_access_token(value: &str) -> Option<String> {
    let value = value.trim();
    if !value.contains('=') {
        return (!value.is_empty()).then(|| value.trim_start_matches("Bearer ").to_owned());
    }
    value.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        matches!(key.trim(), "access_token" | "accessToken" | "token")
            .then(|| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

fn login_required() -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::LoginRequired,
        provider: ProviderId::Spotify,
        message: "spotify OAuth access token or client credentials required".to_owned(),
        retryable: false,
        action: Some("login".to_owned()),
        raw_message: None,
    }
}
fn unavailable(error: reqwest::Error) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::Unavailable,
        provider: ProviderId::Spotify,
        message: error.to_string(),
        retryable: true,
        action: None,
        raw_message: None,
    }
}
fn invalid_response(message: String, raw_message: String) -> ProviderError {
    ProviderError {
        code: ProviderErrorCode::InvalidResponse,
        provider: ProviderId::Spotify,
        message,
        retryable: false,
        action: None,
        raw_message: Some(raw_message),
    }
}
fn spotify_error(status: StatusCode, raw_message: String) -> ProviderError {
    let message = serde_json::from_str::<Value>(&raw_message)
        .ok()
        .and_then(|body| {
            body.pointer("/error/message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| format!("spotify upstream http {}", status.as_u16()));
    let code = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ProviderErrorCode::LoginRequired,
        StatusCode::NOT_FOUND => ProviderErrorCode::NoResult,
        StatusCode::TOO_MANY_REQUESTS
        | StatusCode::INTERNAL_SERVER_ERROR
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE => ProviderErrorCode::Unavailable,
        _ => ProviderErrorCode::InvalidResponse,
    };
    ProviderError {
        code,
        provider: ProviderId::Spotify,
        message,
        retryable: matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS
                | StatusCode::INTERNAL_SERVER_ERROR
                | StatusCode::BAD_GATEWAY
                | StatusCode::SERVICE_UNAVAILABLE
        ),
        action: None,
        raw_message: Some(raw_message),
    }
}

#[cfg(test)]
mod tests {
    use super::extract_access_token;
    #[test]
    fn extracts_session_token() {
        assert_eq!(
            extract_access_token("access_token=abc; refresh_token=def"),
            Some("abc".to_owned())
        );
        assert_eq!(extract_access_token("Bearer abc"), Some("abc".to_owned()));
    }
}
