use std::{env, io::Read, sync::Arc};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, Method, StatusCode};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    librespot_audio::{AudioDecrypt, AudioFile},
    librespot_core::{
        FileId, Session, SpotifyId, SpotifyUri, authentication::Credentials, cdn_url::CdnUrl,
        config::SessionConfig, error::ErrorKind,
    },
    librespot_metadata::audio::{AudioFileFormat, AudioFiles, AudioItem},
    providers::{
        ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    services::auth_session,
    types::{ProviderId, TrackQualityOption},
};

const API_BASE: &str = "https://api.spotify.com/v1";
const ACCOUNTS_BASE: &str = "https://accounts.spotify.com";

#[derive(Clone, Debug)]
struct ClientToken {
    value: String,
    expires_at: std::time::Instant,
}

#[derive(Clone)]
struct SpotifySession {
    access_token: String,
    session: Session,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpotifyAudioQuality {
    pub id: &'static str,
    pub label: &'static str,
    pub short: &'static str,
    pub br: u32,
    pub format: AudioFileFormat,
}

#[derive(Clone, Debug)]
pub struct SpotifyResolvedAudio {
    pub file_id: FileId,
    pub format: SpotifyAudioQuality,
}

pub struct SpotifyAudioBytes {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub quality: SpotifyAudioQuality,
}

const SPOTIFY_OGG_HEADER_END: u64 = 0xa7;

const SPOTIFY_AUDIO_QUALITIES: [SpotifyAudioQuality; 3] = [
    SpotifyAudioQuality {
        id: "high",
        label: "Ogg Vorbis 320k",
        short: "320k",
        br: 320_000,
        format: AudioFileFormat::OGG_VORBIS_320,
    },
    SpotifyAudioQuality {
        id: "medium",
        label: "Ogg Vorbis 160k",
        short: "160k",
        br: 160_000,
        format: AudioFileFormat::OGG_VORBIS_160,
    },
    SpotifyAudioQuality {
        id: "low",
        label: "Ogg Vorbis 96k",
        short: "96k",
        br: 96_000,
        format: AudioFileFormat::OGG_VORBIS_96,
    },
];

#[derive(Clone)]
pub struct SpotifyClient {
    http: Client,
    client_id: String,
    client_secret: String,
    market: String,
    client_token: Arc<Mutex<Option<ClientToken>>>,
    playback_session: Arc<Mutex<Option<SpotifySession>>>,
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
            playback_session: Arc::new(Mutex::new(None)),
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
        if let Some(session) = self.playback_session.lock().await.take() {
            session.session.shutdown();
        }
    }

    pub async fn available_qualities(
        &self,
        track_id: &str,
    ) -> ProviderResult<Vec<TrackQualityOption>> {
        let session = self.playback_session().await?;
        let track = self
            .load_audio_item(&session, spotify_track_uri(track_id)?)
            .await?;
        let qualities = SPOTIFY_AUDIO_QUALITIES
            .iter()
            .filter(|quality| track.files.contains_key(&quality.format))
            .map(|quality| spotify_quality_option(track_id, *quality))
            .collect::<Vec<_>>();
        if qualities.is_empty() {
            return Err(ProviderError {
                code: ProviderErrorCode::NoUrl,
                provider: ProviderId::Spotify,
                message: format!("spotify track {track_id} has no supported audio files"),
                retryable: false,
                action: None,
                raw_message: None,
            });
        }
        Ok(qualities)
    }

    pub async fn resolve_audio(
        &self,
        track_id: &str,
        requested_quality: Option<&str>,
    ) -> ProviderResult<SpotifyResolvedAudio> {
        let session = self.playback_session().await?;
        let audio = self
            .load_audio_item(&session, spotify_track_uri(track_id)?)
            .await?;
        let (format, file_id) = choose_audio_file(&audio, requested_quality)?;
        CdnUrl::new(file_id)
            .resolve_audio(&session)
            .await
            .and_then(|cdn| cdn.try_get_urls().map(|urls| urls[0].to_owned()))
            .map_err(librespot_error)?;
        Ok(SpotifyResolvedAudio { file_id, format })
    }

    pub async fn audio_bytes(
        &self,
        track_id: &str,
        requested_quality: Option<&str>,
    ) -> ProviderResult<SpotifyAudioBytes> {
        let session = self.playback_session().await?;
        let track_uri = spotify_track_uri(track_id)?;
        let track_spotify_id = SpotifyId::try_from(&track_uri).map_err(librespot_error)?;
        let audio = self.load_audio_item(&session, track_uri).await?;
        let (quality, file_id) = choose_audio_file(&audio, requested_quality)?;
        let encrypted_file = AudioFile::open(&session, file_id, stream_data_rate(quality.format))
            .await
            .map_err(librespot_error)?;
        let key = session
            .audio_key()
            .request(track_spotify_id, file_id)
            .await
            .ok();
        let content_type = audio_mime_type(quality.format);
        let bytes = tokio::task::spawn_blocking(move || {
            let mut decrypted = AudioDecrypt::new(key, encrypted_file);
            if AudioFiles::is_ogg_vorbis(quality.format) {
                std::io::Seek::seek(
                    &mut decrypted,
                    std::io::SeekFrom::Start(SPOTIFY_OGG_HEADER_END),
                )?;
            }
            let mut bytes = Vec::new();
            decrypted.read_to_end(&mut bytes)?;
            Ok::<_, std::io::Error>(bytes)
        })
        .await
        .map_err(|error| ProviderError {
            code: ProviderErrorCode::Internal,
            provider: ProviderId::Spotify,
            message: format!("spotify audio worker failed: {error}"),
            retryable: false,
            action: None,
            raw_message: None,
        })?
        .map_err(|error| ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: ProviderId::Spotify,
            message: format!("read spotify audio: {error}"),
            retryable: true,
            action: None,
            raw_message: None,
        })?;
        Ok(SpotifyAudioBytes {
            bytes,
            content_type,
            quality,
        })
    }

    async fn playback_session(&self) -> ProviderResult<Session> {
        self.ensure_premium().await?;
        let access_token = self.user_access_token().await?;
        let mut cached = self.playback_session.lock().await;
        if let Some(existing) = cached.as_ref().filter(|existing| {
            existing.access_token == access_token && !existing.session.is_invalid()
        }) {
            return Ok(existing.session.clone());
        }
        let session = Session::new(SessionConfig::default(), None);
        session
            .connect(Credentials::with_access_token(access_token.clone()), false)
            .await
            .map_err(librespot_error)?;
        *cached = Some(SpotifySession {
            access_token,
            session: session.clone(),
        });
        Ok(session)
    }

    async fn ensure_premium(&self) -> ProviderResult<()> {
        let body = self.request(Method::GET, "me", &[], None, true).await?;
        let premium = body
            .get("product")
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("premium"));
        if premium {
            return Ok(());
        }
        Err(ProviderError {
            code: ProviderErrorCode::VipRequired,
            provider: ProviderId::Spotify,
            message: "spotify playback requires a Premium account".to_owned(),
            retryable: false,
            action: Some("login".to_owned()),
            raw_message: None,
        })
    }

    async fn load_audio_item(
        &self,
        session: &Session,
        uri: SpotifyUri,
    ) -> ProviderResult<AudioItem> {
        let audio = AudioItem::get_file(session, uri)
            .await
            .map_err(librespot_error)?;
        if audio.availability.is_ok() && !audio.files.is_empty() {
            return Ok(audio);
        }
        if let Some(alternatives) = audio.alternatives {
            for alternative in alternatives.0 {
                if let Ok(audio) = AudioItem::get_file(session, alternative).await {
                    if audio.availability.is_ok() && !audio.files.is_empty() {
                        return Ok(audio);
                    }
                }
            }
        }
        Err(ProviderError {
            code: ProviderErrorCode::CopyrightUnavailable,
            provider: ProviderId::Spotify,
            message: "spotify track is unavailable for this account or market".to_owned(),
            retryable: false,
            action: None,
            raw_message: None,
        })
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

fn spotify_track_uri(track_id: &str) -> ProviderResult<SpotifyUri> {
    let value = track_id.trim();
    if value.starts_with("spotify:track:") {
        return SpotifyUri::from_uri(value).map_err(librespot_error);
    }
    SpotifyId::from_base62(value)
        .map(|id| SpotifyUri::Track { id })
        .map_err(librespot_error)
}

fn choose_audio_file(
    audio: &AudioItem,
    requested_quality: Option<&str>,
) -> ProviderResult<(SpotifyAudioQuality, FileId)> {
    let start = requested_quality
        .and_then(normalize_quality)
        .and_then(|quality_id| {
            SPOTIFY_AUDIO_QUALITIES
                .iter()
                .position(|quality| quality.id == quality_id)
        })
        .unwrap_or(0);
    SPOTIFY_AUDIO_QUALITIES
        .iter()
        .skip(start)
        .chain(SPOTIFY_AUDIO_QUALITIES.iter().take(start))
        .find_map(|quality| {
            audio
                .files
                .get(&quality.format)
                .copied()
                .map(|file_id| (*quality, file_id))
        })
        .ok_or_else(|| ProviderError {
            code: ProviderErrorCode::NoUrl,
            provider: ProviderId::Spotify,
            message: format!("spotify track {} has no supported audio file", audio.uri),
            retryable: false,
            action: None,
            raw_message: None,
        })
}

fn normalize_quality(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "high" | "320" | "320k" | "exhigh" | "hires" | "lossless" => Some("high"),
        "medium" | "160" | "160k" | "standard" => Some("medium"),
        "low" | "96" | "96k" => Some("low"),
        _ => None,
    }
}

fn spotify_quality_option(track_id: &str, quality: SpotifyAudioQuality) -> TrackQualityOption {
    TrackQualityOption {
        provider: ProviderId::Spotify,
        id: quality.id.to_owned(),
        label: quality.label.to_owned(),
        short: Some(quality.short.to_owned()),
        detail: Some("Spotify Premium via librespot".to_owned()),
        request_quality: quality.id.to_owned(),
        level: Some(quality.id.to_owned()),
        r#type: Some("ogg".to_owned()),
        br: Some(quality.br),
        size: None,
        format: Some("ogg".to_owned()),
        source: format!("spotify:{track_id}"),
    }
}

fn stream_data_rate(format: AudioFileFormat) -> usize {
    let kbps: f32 = match format {
        AudioFileFormat::OGG_VORBIS_96 | AudioFileFormat::MP3_96 => 12.,
        AudioFileFormat::OGG_VORBIS_160
        | AudioFileFormat::MP3_160
        | AudioFileFormat::MP3_160_ENC
        | AudioFileFormat::AAC_160 => 20.,
        AudioFileFormat::OGG_VORBIS_320
        | AudioFileFormat::MP3_320
        | AudioFileFormat::AAC_320
        | AudioFileFormat::OTHER5 => 40.,
        AudioFileFormat::MP3_256 => 32.,
        AudioFileFormat::AAC_24
        | AudioFileFormat::XHE_AAC_24
        | AudioFileFormat::FLAC_FLAC_24BIT => 3.,
        AudioFileFormat::AAC_48 => 6.,
        AudioFileFormat::FLAC_FLAC => 112.,
        AudioFileFormat::XHE_AAC_12 => 1.5,
        AudioFileFormat::XHE_AAC_16 => 2.,
        AudioFileFormat::MP4_128 => 16.,
    };
    (kbps * 1024.).ceil() as usize
}

fn audio_mime_type(format: AudioFileFormat) -> &'static str {
    AudioFiles::mime_type(format).unwrap_or("application/octet-stream")
}

fn librespot_error(error: crate::librespot_core::Error) -> ProviderError {
    let code = match error.kind {
        ErrorKind::Unauthenticated | ErrorKind::PermissionDenied => {
            ProviderErrorCode::LoginRequired
        }
        ErrorKind::NotFound => ProviderErrorCode::NoResult,
        ErrorKind::InvalidArgument => ProviderErrorCode::InvalidResponse,
        ErrorKind::FailedPrecondition => ProviderErrorCode::CopyrightUnavailable,
        ErrorKind::Unavailable
        | ErrorKind::DeadlineExceeded
        | ErrorKind::ResourceExhausted
        | ErrorKind::Aborted => ProviderErrorCode::Unavailable,
        _ => ProviderErrorCode::Internal,
    };
    ProviderError {
        code,
        provider: ProviderId::Spotify,
        message: error.to_string(),
        retryable: matches!(
            error.kind,
            ErrorKind::Unavailable
                | ErrorKind::DeadlineExceeded
                | ErrorKind::ResourceExhausted
                | ErrorKind::Aborted
        ),
        action: None,
        raw_message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_access_token, normalize_quality};
    #[test]
    fn extracts_session_token() {
        assert_eq!(
            extract_access_token("access_token=abc; refresh_token=def"),
            Some("abc".to_owned())
        );
        assert_eq!(extract_access_token("Bearer abc"), Some("abc".to_owned()));
    }

    #[test]
    fn normalizes_spotify_quality_aliases() {
        assert_eq!(normalize_quality("320k"), Some("high"));
        assert_eq!(normalize_quality("standard"), Some("medium"));
        assert_eq!(normalize_quality("96"), Some("low"));
    }
}
