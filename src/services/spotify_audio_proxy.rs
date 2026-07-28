use std::sync::Arc;

use axum::{
    body::Body,
    http::{HeaderValue, StatusCode, header},
    response::Response,
};

use crate::{
    http::response::fail,
    providers::{
        error::{ProviderError, ProviderErrorCode},
        spotify::client::SpotifyClient,
    },
};

#[derive(Clone)]
pub struct SpotifyAudioProxy {
    client: Arc<SpotifyClient>,
}

pub struct SpotifyAudioProxyRequest {
    pub track_id: String,
    pub quality: Option<String>,
}

impl SpotifyAudioProxy {
    pub fn new(client: Arc<SpotifyClient>) -> Self {
        Self { client }
    }

    pub async fn resolve(&self, input: SpotifyAudioProxyRequest) -> Response {
        if input.track_id.trim().is_empty() {
            return fail(StatusCode::BAD_REQUEST, "BAD_REQUEST", "id required");
        }
        match self
            .client
            .audio_bytes(&input.track_id, input.quality.as_deref())
            .await
        {
            Ok(audio) => Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, audio.content_type)
                .header(header::CACHE_CONTROL, "private, max-age=0, no-store")
                .header(header::ACCEPT_RANGES, "none")
                .header("access-control-allow-origin", HeaderValue::from_static("*"))
                .header("x-spotify-quality", audio.quality.id)
                .body(Body::from(audio.bytes))
                .unwrap_or_else(|_| fail(StatusCode::BAD_GATEWAY, "SPOTIFY_AUDIO", "audio failed")),
            Err(err) => spotify_error_response(err),
        }
    }
}

pub fn create_spotify_audio_proxy(client: Arc<SpotifyClient>) -> SpotifyAudioProxy {
    SpotifyAudioProxy::new(client)
}

fn spotify_error_response(err: ProviderError) -> Response {
    let status = match err.code {
        ProviderErrorCode::LoginRequired => StatusCode::UNAUTHORIZED,
        ProviderErrorCode::NoResult | ProviderErrorCode::NoUrl => StatusCode::NOT_FOUND,
        ProviderErrorCode::VipRequired | ProviderErrorCode::PaidRequired => StatusCode::FORBIDDEN,
        ProviderErrorCode::InvalidResponse => StatusCode::BAD_GATEWAY,
        ProviderErrorCode::Unavailable
        | ProviderErrorCode::CopyrightUnavailable
        | ProviderErrorCode::TrialOnly => StatusCode::BAD_GATEWAY,
        ProviderErrorCode::NotImplemented => StatusCode::NOT_IMPLEMENTED,
        ProviderErrorCode::Internal | ProviderErrorCode::NoPlaylist => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    fail(
        status,
        format!("{:?}", err.code).to_uppercase(),
        err.message,
    )
}
