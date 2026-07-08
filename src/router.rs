use axum::{
    body::Body,
    Router,
    extract::{Path, Query, Request, State},
    http::{Method, StatusCode},
    response::Response,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;

use crate::{
    http::response::{cors_preflight, fail, json, ok},
    providers::{
        error::{ProviderError, ProviderErrorCode},
        registry::ProviderRegistry,
    },
    providers::registry::{CapabilityMatrix, PROVIDER_IDS, build_capability_matrix},
    server::AppState,
    services::{self, cross_source_resolver, weather_radio::WeatherRadioParams},
    types::{SongUrlOptions, Track},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    ok: bool,
    app_version: String,
    api_version: String,
    schema_version: String,
    providers: Vec<&'static str>,
    provider_status: CapabilityMatrix,
}

pub fn build(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health).options(preflight))
        .route(
            "/providers/capabilities",
            get(provider_capabilities).options(preflight),
        )
        .route("/diagnostics", get(diagnostics).options(preflight))
        .route("/audio-proxy", get(audio_proxy).options(preflight))
        .route("/image-proxy", get(image_proxy).options(preflight))
        .route(
            "/providers/soda/audio-proxy",
            get(soda_audio_proxy).options(preflight),
        )
        .route("/weather/radio", get(weather_radio).options(preflight))
        .route("/search", get(search).options(preflight))
        .route("/song-url", post(song_url).options(preflight))
        .route(
            "/shared-playlist/import",
            post(shared_playlist_import).options(preflight),
        )
        .route(
            "/providers/:pid/login-qr-key",
            get(provider_login_qr_key).options(preflight),
        )
        .route(
            "/providers/:pid/login-qr-create",
            get(provider_login_qr_create).options(preflight),
        )
        .route(
            "/providers/:pid/login-qr-check",
            get(provider_login_qr_check).options(preflight),
        )
        .route(
            "/providers/:pid/session-cookie",
            post(set_provider_session_cookie)
                .delete(clear_provider_session_cookie)
                .options(preflight),
        )
        .route(
            "/providers/:pid/search",
            get(provider_search).options(preflight),
        )
        .route(
            "/providers/:pid/song-url",
            post(provider_song_url).options(preflight),
        )
        .route(
            "/providers/:pid/qualities",
            post(provider_qualities).options(preflight),
        )
        .route(
            "/providers/:pid/lyric",
            post(provider_lyric).options(preflight),
        )
        .route(
            "/providers/:pid/playlists",
            get(provider_playlists).options(preflight),
        )
        .route(
            "/providers/:pid/playlists/:id",
            get(provider_playlist_detail).options(preflight),
        )
        .route(
            "/providers/:pid/login-status",
            get(provider_login_status).options(preflight),
        )
        .route("/providers/:pid/logout", post(provider_logout).options(preflight))
        .route("/providers/:pid/like", post(provider_like).options(preflight))
        .route(
            "/providers/:pid/like-check",
            get(provider_like_check).options(preflight),
        )
        .route(
            "/providers/:pid/playlists/add-song",
            post(provider_playlist_add_song).options(preflight),
        )
        .fallback(fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    json(
        HealthResponse {
            ok: true,
            app_version: state.config.app_version,
            api_version: state.config.api_version,
            schema_version: state.config.schema_version,
            providers: PROVIDER_IDS.to_vec(),
            provider_status: build_capability_matrix(),
        },
        StatusCode::OK,
    )
}

async fn provider_capabilities(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    ok(state.providers.build_capability_matrix())
}

async fn diagnostics(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    ok(services::diagnostics::snapshot(&state))
}

#[derive(Debug, Deserialize)]
struct ProxyQuery {
    #[serde(alias = "target")]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SodaAudioProxyQuery {
    #[serde(flatten)]
    proxy: ProxyQuery,
    #[serde(alias = "playAuth")]
    play_auth: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    keyword: Option<String>,
    q: Option<String>,
    provider: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SongUrlRequest {
    track: Track,
    options: Option<SongUrlOptions>,
    opts: Option<SongUrlOptions>,
}

#[derive(Debug, Deserialize)]
struct LoginQrQuery {
    key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionCookieRequest {
    cookie: String,
}

#[derive(Debug, Deserialize)]
struct LikeBody {
    id: String,
    liked: bool,
}

#[derive(Debug, Deserialize)]
struct LikeCheckQuery {
    ids: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlaylistAddSongBody {
    playlist_id: String,
    track_id: String,
}

#[derive(Debug, Deserialize)]
struct TrackBody {
    track: Track,
}

async fn audio_proxy(
    State(state): State<AppState>,
    Query(query): Query<ProxyQuery>,
    request: Request,
) -> Response {
    let target = proxy_target(query);
    state
        .services
        .audio_proxy
        .resolve(services::audio_proxy::AudioProxyRequest {
            target,
            request: request.map(Body::new),
        })
        .await
}

async fn image_proxy(
    State(state): State<AppState>,
    Query(query): Query<ProxyQuery>,
    request: Request,
) -> Response {
    let target = proxy_target(query);
    state
        .services
        .image_proxy
        .resolve(services::image_proxy::ImageProxyRequest {
            target,
            request: request.map(Body::new),
        })
        .await
}

async fn soda_audio_proxy(
    State(state): State<AppState>,
    Query(query): Query<SodaAudioProxyQuery>,
    request: Request,
) -> Response {
    let target = proxy_target(query.proxy);
    state
        .services
        .soda_audio_proxy
        .resolve(services::soda_audio_proxy::SodaAudioProxyRequest {
            target,
            request: request.map(Body::new),
            play_auth: query.play_auth,
        })
        .await
}

async fn weather_radio(
    State(state): State<AppState>,
    Query(params): Query<WeatherRadioParams>,
) -> Response {
    match state.services.weather_radio.build(params).await {
        Ok(value) => ok(value),
        Err(err) => internal_error(err.to_string()),
    }
}

async fn search(State(state): State<AppState>, Query(query): Query<SearchQuery>) -> Response {
    let keyword = search_keyword(&query);
    if keyword.is_empty() {
        return bad_request("keyword required");
    }

    let resolver = build_cross_source_resolver(&state.providers);
    match resolver
        .resolve_search(cross_source_resolver::ResolveSearchQuery {
            keyword,
            provider: query.provider.filter(|value| !value.trim().is_empty()),
            limit: query.limit.unwrap_or(20).max(1),
        })
        .await
    {
        Ok(tracks) => ok(tracks),
        Err(err) => anyhow_error_response(err),
    }
}

async fn song_url(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<SongUrlRequest>,
) -> Response {
    let resolver = build_cross_source_resolver(&state.providers);
    match resolver
        .resolve_song_url(body.track, body.options.or(body.opts))
        .await
    {
        Ok(result) => ok(result),
        Err(err) => anyhow_error_response(err),
    }
}

async fn shared_playlist_import(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    match services::shared_playlist_import::import_shared_playlist(
        body,
        services::shared_playlist_import::SharedPlaylistImporterDeps {
            provider_adapters: state.providers.all(),
        },
    )
    .await
    {
        Ok(result) => ok(result),
        Err(err) => match err.downcast::<services::shared_playlist_import::SharedPlaylistImportError>() {
            Ok(err) => {
                let status = match err.code.as_str() {
                    "UNSUPPORTED_LINK" => StatusCode::BAD_REQUEST,
                    "UNSUPPORTED_PROVIDER" | "NOT_IMPLEMENTED" => StatusCode::NOT_IMPLEMENTED,
                    _ => StatusCode::INTERNAL_SERVER_ERROR,
                };
                fail(status, err.code, err.message)
            }
            Err(err) => anyhow_error_response(err),
        },
    }
}

async fn provider_login_qr_key(
    State(state): State<AppState>,
    Path(pid): Path<String>,
) -> Response {
    match pid.as_str() {
        "qq" => match state.services.qq_qr_login.create_key().await {
            Ok(data) => ok(data),
            Err(err) => internal_error(err.to_string()),
        },
        "soda" => match state.services.soda_qr_login.create_image(None).await {
            Ok(image) => ok(crate::types::ProviderLoginQrKey {
                provider: image.provider,
                key: image.key,
            }),
            Err(err) => internal_error(err.to_string()),
        },
        "netease" => not_implemented("netease QR login still needs a wired API client"),
        _ => unknown_provider(&pid),
    }
}

async fn provider_login_qr_create(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    Query(query): Query<LoginQrQuery>,
) -> Response {
    let key = query.key.unwrap_or_default();
    match pid.as_str() {
        "qq" => match state.services.qq_qr_login.create_image(&key).await {
            Ok(data) => ok(data),
            Err(err) => bad_request(err.to_string()),
        },
        "soda" => match state.services.soda_qr_login.create_image(Some(&key)).await {
            Ok(data) => ok(data),
            Err(err) => bad_request(err.to_string()),
        },
        "netease" => not_implemented("netease QR login still needs a wired API client"),
        _ => unknown_provider(&pid),
    }
}

async fn provider_login_qr_check(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    Query(query): Query<LoginQrQuery>,
) -> Response {
    let key = query.key.unwrap_or_default();
    match pid.as_str() {
        "qq" => match state.services.qq_qr_login.check(&key).await {
            Ok(data) => ok(data),
            Err(err) => bad_request(err.to_string()),
        },
        "soda" => match state.services.soda_qr_login.check(&key).await {
            Ok(data) => ok(data),
            Err(err) => bad_request(err.to_string()),
        },
        "netease" => not_implemented("netease QR login still needs a wired API client"),
        _ => unknown_provider(&pid),
    }
}

async fn set_provider_session_cookie(
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<SessionCookieRequest>,
) -> Response {
    if !is_known_provider(&pid) {
        return unknown_provider(&pid);
    }
    match services::auth_session::set_runtime_provider_cookie(pid, body.cookie).await {
        Ok(()) => ok(serde_json::json!({ "stored": true })),
        Err(err) => bad_request(err),
    }
}

async fn clear_provider_session_cookie(Path(pid): Path<String>) -> Response {
    if !is_known_provider(&pid) {
        return unknown_provider(&pid);
    }
    services::auth_session::clear_runtime_provider_cookie(&pid).await;
    ok(serde_json::json!({ "stored": false }))
}

async fn provider_search(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Response {
    let keyword = search_keyword(&query);
    if keyword.is_empty() {
        return bad_request("keyword required");
    }
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider.search(&keyword, query.limit.unwrap_or(20).max(1)).await {
        Ok(tracks) => ok(tracks),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_song_url(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<SongUrlRequest>,
) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider.song_url(&body.track, body.options.or(body.opts)).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_qualities(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<TrackBody>,
) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider.track_qualities(&body.track).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_lyric(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<TrackBody>,
) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider.lyric(&body.track).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_playlists(State(state): State<AppState>, Path(pid): Path<String>) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider.playlist_list().await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_playlist_detail(
    State(state): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider.playlist_detail(&id).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_login_status(
    State(state): State<AppState>,
    Path(pid): Path<String>,
) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider.login_status().await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_logout(State(state): State<AppState>, Path(pid): Path<String>) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider.logout().await {
        Ok(()) => ok(serde_json::json!({ "ok": true })),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_like(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<LikeBody>,
) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider.like_song(&body.id, body.liked).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_like_check(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    Query(query): Query<LikeCheckQuery>,
) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    let ids = query
        .ids
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    match provider.check_song_likes(&ids).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_playlist_add_song(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<PlaylistAddSongBody>,
) -> Response {
    let Some(provider) = state.providers.get(&pid) else {
        return unavailable_provider(&pid);
    };
    match provider
        .add_song_to_playlist(&body.playlist_id, &body.track_id)
        .await
    {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn preflight() -> impl axum::response::IntoResponse {
    cors_preflight()
}

async fn fallback(request: axum::extract::Request) -> Response {
    if request.method() == Method::OPTIONS {
        return cors_preflight();
    }

    fail(
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
        format!(
            "unknown route: {} {}",
            request.method(),
            request.uri().path()
        ),
    )
}

fn proxy_target(query: ProxyQuery) -> String {
    query.url.unwrap_or_default()
}

fn search_keyword(query: &SearchQuery) -> String {
    query
        .keyword
        .clone()
        .or_else(|| query.q.clone())
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn build_cross_source_resolver(
    registry: &ProviderRegistry,
) -> cross_source_resolver::CrossSourceResolver {
    cross_source_resolver::create_cross_source_resolver(
        cross_source_resolver::CrossSourceResolverDeps {
            providers: Some(registry.all()),
            provider_order: None,
        },
    )
}

fn is_known_provider(provider: &str) -> bool {
    matches!(provider, "netease" | "qq" | "soda")
}

fn unknown_provider(provider: &str) -> Response {
    fail(
        StatusCode::NOT_FOUND,
        "PROVIDER_NOT_FOUND",
        format!("unknown provider: {provider}"),
    )
}

fn unavailable_provider(provider: &str) -> Response {
    if !is_known_provider(provider) {
        return unknown_provider(provider);
    }
    fail(
        StatusCode::NOT_IMPLEMENTED,
        "PROVIDER_UNAVAILABLE",
        format!("provider {provider} is not wired into the registry yet"),
    )
}

fn not_implemented(message: impl Into<String>) -> Response {
    fail(StatusCode::NOT_IMPLEMENTED, "NOT_IMPLEMENTED", message)
}

fn bad_request(message: impl Into<String>) -> Response {
    fail(StatusCode::BAD_REQUEST, "BAD_REQUEST", message)
}

fn internal_error(message: impl Into<String>) -> Response {
    fail(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", message)
}

fn provider_error_response(err: ProviderError) -> Response {
    let status = match err.code {
        ProviderErrorCode::LoginRequired => StatusCode::UNAUTHORIZED,
        ProviderErrorCode::NotImplemented => StatusCode::NOT_IMPLEMENTED,
        ProviderErrorCode::NoResult
        | ProviderErrorCode::NoUrl
        | ProviderErrorCode::NoLyric
        | ProviderErrorCode::NoPlaylists
        | ProviderErrorCode::NoPlaylist => StatusCode::NOT_FOUND,
        ProviderErrorCode::Unavailable
        | ProviderErrorCode::CopyrightUnavailable
        | ProviderErrorCode::PaidRequired
        | ProviderErrorCode::TrialOnly
        | ProviderErrorCode::VipRequired => StatusCode::BAD_GATEWAY,
        ProviderErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    fail(status, format!("{:?}", err.code).to_uppercase(), err.message)
}

fn anyhow_error_response(err: anyhow::Error) -> Response {
    match err.downcast::<ProviderError>() {
        Ok(provider_err) => provider_error_response(provider_err),
        Err(err) => internal_error(err.to_string()),
    }
}
