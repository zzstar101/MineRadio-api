use axum::{
    Router,
    body::Body,
    extract::{Path, Query, Request, State},
    http::{Method, StatusCode},
    response::Response,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    http::response::{cors_preflight, fail, json, ok},
    providers::registry::{CapabilityMatrix, PROVIDER_IDS, build_capability_matrix},
    providers::{
        error::{ProviderError, ProviderErrorCode},
    },
    server::AppState,
    services::{
        self, podcast,
        qr_login::{QrLogin, QrLoginKind},
        sidecar_log,
        weather_radio::WeatherRadioParams,
    },
    types::{ProviderId, SearchType, SongUrlOptions, Track},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    ok: bool,
    app_version: String,
    api_version: String,
    schema_version: String,
    providers: Vec<ProviderId>,
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
        .route(
            "/providers/spotify/audio-proxy",
            get(spotify_audio_proxy).options(preflight),
        )
        .route(
            "/providers/qq/audio-proxy",
            get(qq_audio_proxy_handler).options(preflight),
        )
        .route("/weather/radio", get(weather_radio).options(preflight))
        .route("/discover/home", get(discover_home).options(preflight))
        .route("/podcast/search", get(podcast_search).options(preflight))
        .route("/podcast/hot", get(podcast_hot).options(preflight))
        .route("/podcast/detail", get(podcast_detail).options(preflight))
        .route(
            "/podcast/programs",
            get(podcast_programs).options(preflight),
        )
        .route("/podcast/my", get(podcast_my).options(preflight))
        .route(
            "/podcast/my/items",
            get(podcast_my_items).options(preflight),
        )
        .route(
            "/podcast/dj-beatmap",
            get(podcast_dj_beatmap).options(preflight),
        )
        .route(
            "/shared-playlist/import",
            post(shared_playlist_import).options(preflight),
        )
        .route(
            "/providers/{pid}/login-qr-key",
            get(provider_login_qr_key).options(preflight),
        )
        .route(
            "/providers/{pid}/login-qr-create",
            get(provider_login_qr_create).options(preflight),
        )
        .route(
            "/providers/{pid}/login-qr-check",
            get(provider_login_qr_check).options(preflight),
        )
        .route(
            "/providers/{pid}/session-cookie",
            post(set_provider_session_cookie)
                .delete(clear_provider_session_cookie)
                .options(preflight),
        )
        .route(
            "/providers/{pid}/session-cookie/clear",
            post(clear_provider_session_cookie).options(preflight),
        )
        .route(
            "/providers/{pid}/search",
            get(provider_search).options(preflight),
        )
        .route(
            "/providers/{pid}/song-url",
            post(provider_song_url).options(preflight),
        )
        .route(
            "/providers/{pid}/qualities",
            post(provider_qualities).options(preflight),
        )
        .route(
            "/providers/{pid}/lyric",
            post(provider_lyric).options(preflight),
        )
        .route(
            "/providers/{pid}/playlists",
            get(provider_playlists).options(preflight),
        )
        .route(
            "/providers/{pid}/playlists/{id}",
            get(provider_playlist_detail).options(preflight),
        )
        .route(
            "/providers/{pid}/albums",
            get(provider_albums).options(preflight),
        )
        .route(
            "/providers/{pid}/albums/{id}",
            get(provider_album_detail).options(preflight),
        )
        .route(
            "/providers/{pid}/login-status",
            get(provider_login_status).options(preflight),
        )
        .route(
            "/providers/{pid}/logout",
            post(provider_logout).options(preflight),
        )
        .route(
            "/providers/{pid}/like",
            post(provider_like).options(preflight),
        )
        .route(
            "/providers/{pid}/like-check",
            get(provider_like_check).options(preflight),
        )
        .route(
            "/providers/{pid}/playlists/add-song",
            post(provider_playlist_add_song).options(preflight),
        )
        .route(
            "/providers/{pid}/playlists/del-song",
            post(provider_playlist_del_song).options(preflight),
        )
        .fallback(fallback)
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
struct SpotifyAudioProxyQuery {
    id: Option<String>,
    quality: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    keyword: Option<String>,
    q: Option<String>,
    provider: Option<String>,
    #[serde(rename = "type")]
    search_type: Option<SearchType>,
    offset: Option<u32>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OffsetLimitQuery {
    offset: Option<u32>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SongUrlTrackQualityRequest {
    track: Track,
    quality: Option<String>,
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
struct PodcastSearchQuery {
    keywords: Option<String>,
    keyword: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct PodcastPageQuery {
    limit: Option<u32>,
    offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct PodcastDetailQuery {
    id: Option<String>,
    rid: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PodcastMyItemsQuery {
    key: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct PodcastProgramsQuery {
    id: Option<String>,
    rid: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct PodcastBeatmapQuery {
    url: Option<String>,
    duration: Option<u32>,
    intro: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct LikeBody {
    id: String,
    liked: bool,
}

#[derive(Debug, Deserialize)]
struct LikeCheckQuery {
    ids: Option<String>,
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PlaylistAddSongBody {
    playlist_id: String,
    track_id: String,
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
    _request: Request,
) -> Response {
    let target = proxy_target(query);
    state
        .services
        .image_proxy
        .resolve(services::image_proxy::ImageProxyRequest { target })
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

async fn spotify_audio_proxy(
    State(state): State<AppState>,
    Query(query): Query<SpotifyAudioProxyQuery>,
) -> Response {
    state
        .services
        .spotify_audio_proxy
        .resolve(services::spotify_audio_proxy::SpotifyAudioProxyRequest {
            track_id: query.id.unwrap_or_default(),
            quality: query.quality,
        })
        .await
}

async fn qq_audio_proxy_handler(
    State(state): State<AppState>,
    Query(query): Query<ProxyQuery>,
    request: Request,
) -> Response {
    let target = proxy_target(query);
    let cookie = services::auth_session::get_provider_cookie(&ProviderId::Qq).await;
    state
        .services
        .qq_audio_proxy
        .resolve(services::qq_audio_proxy::QqAudioProxyRequest {
            target,
            request: request.map(Body::new),
            ekey: None,
            cookie,
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

async fn discover_home(State(state): State<AppState>) -> Response {
    match services::discover_home::build_discover_home(
        services::discover_home::DiscoverHomeServiceOptions {
            provider_adapters: state.providers.all(),
            podcast: state.services.podcast.clone(),
            discover_requester: Some(state.services.discover_requester.clone()),
        },
    )
    .await
    {
        Ok(value) => ok(value),
        Err(err) => internal_error(err.to_string()),
    }
}

async fn podcast_search(
    State(state): State<AppState>,
    Query(query): Query<PodcastSearchQuery>,
) -> Response {
    let keywords = query
        .keywords
        .or(query.keyword)
        .unwrap_or_default()
        .trim()
        .to_owned();
    match state
        .services
        .podcast
        .search(podcast::PodcastSearchParams {
            keywords,
            limit: query.limit.unwrap_or(18),
        })
        .await
    {
        Ok(value) => ok(value),
        Err(err) => bad_request(err.to_string()),
    }
}

async fn podcast_hot(
    State(state): State<AppState>,
    Query(query): Query<PodcastPageQuery>,
) -> Response {
    match state
        .services
        .podcast
        .hot(podcast::PodcastPageParams {
            limit: query.limit.unwrap_or(18),
            offset: query.offset.unwrap_or(0),
        })
        .await
    {
        Ok(value) => ok(value),
        Err(err) => internal_error(err.to_string()),
    }
}

async fn podcast_detail(
    State(state): State<AppState>,
    Query(query): Query<PodcastDetailQuery>,
) -> Response {
    match state
        .services
        .podcast
        .detail(podcast::PodcastDetailParams {
            rid: query.id.or(query.rid).unwrap_or_default(),
        })
        .await
    {
        Ok(value) => ok(value),
        Err(err) => bad_request(err.to_string()),
    }
}

async fn podcast_programs(
    State(state): State<AppState>,
    Query(query): Query<PodcastProgramsQuery>,
) -> Response {
    match state
        .services
        .podcast
        .programs(podcast::PodcastProgramsParams {
            rid: query.id.or(query.rid).unwrap_or_default(),
            limit: query.limit.unwrap_or(30),
            offset: query.offset.unwrap_or(0),
        })
        .await
    {
        Ok(value) => ok(value),
        Err(err) => bad_request(err.to_string()),
    }
}

async fn podcast_my(State(state): State<AppState>) -> Response {
    match state.services.podcast.my().await {
        Ok(value) => ok(value),
        Err(err) => internal_error(err.to_string()),
    }
}

async fn podcast_my_items(
    State(state): State<AppState>,
    Query(query): Query<PodcastMyItemsQuery>,
) -> Response {
    match state
        .services
        .podcast
        .my_items(podcast::PodcastMyItemsParams {
            key: query.key.unwrap_or_else(|| "collect".to_owned()),
            limit: query.limit.unwrap_or(36),
            offset: query.offset.unwrap_or(0),
        })
        .await
    {
        Ok(value) => ok(value),
        Err(err) => internal_error(err.to_string()),
    }
}

async fn podcast_dj_beatmap(
    State(state): State<AppState>,
    Query(query): Query<PodcastBeatmapQuery>,
) -> Response {
    match state
        .services
        .podcast
        .dj_beatmap(podcast::PodcastBeatmapParams {
            url: query.url.unwrap_or_default(),
            duration_sec: query.duration.unwrap_or(0),
            intro_sec: query.intro,
        })
        .await
    {
        Ok(value) => ok(value),
        Err(err) if err.to_string() == "Invalid audio url" => bad_request(err.to_string()),
        Err(err) if err.to_string() == "podcast analyzer unavailable" => fail(
            StatusCode::NOT_IMPLEMENTED,
            "NOT_IMPLEMENTED",
            err.to_string(),
        ),
        Err(err) => internal_error(err.to_string()),
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
        Err(err) => {
            match err.downcast::<services::shared_playlist_import::SharedPlaylistImportError>() {
                Ok(err) => {
                    let status = match err.code.as_str() {
                        "UNSUPPORTED_LINK" => StatusCode::BAD_REQUEST,
                        "UNSUPPORTED_PROVIDER" | "NOT_IMPLEMENTED" => StatusCode::NOT_IMPLEMENTED,
                        _ => StatusCode::INTERNAL_SERVER_ERROR,
                    };
                    fail(status, err.code, err.message)
                }
                Err(err) => anyhow_error_response(err),
            }
        }
    }
}

async fn provider_login_qr_key(State(state): State<AppState>, Path(pid): Path<String>) -> Response {
    let Ok(kind) = pid.parse::<QrLoginKind>() else {
        return unknown_provider(&pid);
    };
    let service = qr_login_service(&state, kind);
    match service.create_key().await {
        Ok(data) => ok(data),
        Err(err) if err.to_string() == "KUGOU_QR_LOGIN_NOT_IMPLEMENTED" => fail(
            StatusCode::NOT_IMPLEMENTED,
            "NOT_IMPLEMENTED",
            "kugou qr login adapter is not configured",
        ),
        Err(err) => internal_error(err.to_string()),
    }
}

async fn provider_login_qr_create(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    Query(query): Query<LoginQrQuery>,
) -> Response {
    let key = query.key.unwrap_or_default();
    let Ok(kind) = pid.parse::<QrLoginKind>() else {
        return unknown_provider(&pid);
    };
    let service = qr_login_service(&state, kind);
    match service.create_image(&key).await {
        Ok(data) => ok(data),
        Err(err) if err.to_string() == "KUGOU_QR_LOGIN_NOT_IMPLEMENTED" => fail(
            StatusCode::NOT_IMPLEMENTED,
            "NOT_IMPLEMENTED",
            "kugou qr login adapter is not configured",
        ),
        Err(err) => bad_request(err.to_string()),
    }
}

async fn provider_login_qr_check(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    Query(query): Query<LoginQrQuery>,
) -> Response {
    let key = query.key.unwrap_or_default();
    let Ok(kind) = pid.parse::<QrLoginKind>() else {
        return unknown_provider(&pid);
    };
    let service = qr_login_service(&state, kind);
    match service.check(&key).await {
        Ok(data) => ok(data),
        Err(err) if err.to_string() == "KUGOU_QR_LOGIN_NOT_IMPLEMENTED" => fail(
            StatusCode::NOT_IMPLEMENTED,
            "NOT_IMPLEMENTED",
            "kugou qr login adapter is not configured",
        ),
        Err(err) => bad_request(err.to_string()),
    }
}

fn qr_login_service(state: &AppState, kind: QrLoginKind) -> &dyn QrLogin {
    match kind {
        QrLoginKind::QqWeb => state.services.qq_qr_login.as_ref(),
        QrLoginKind::QqMqtt => state.services.qqmusic_qr_login.as_ref(),
        QrLoginKind::QqWechat => state.services.wechat_qr_login.as_ref(),
        QrLoginKind::Netease => state.services.netease_qr_login.as_ref(),
        QrLoginKind::Soda => state.services.soda_qr_login.as_ref(),
        QrLoginKind::Kugou => state.services.kugou_qr_login.as_ref(),
    }
}

async fn set_provider_session_cookie(
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<SessionCookieRequest>,
) -> Response {
    let Ok(provider) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    if !is_known_provider(provider) {
        return unknown_provider(&pid);
    }
    match services::auth_session::set_runtime_provider_cookie(provider, body.cookie).await {
        Ok(()) => ok(serde_json::json!({ "provider": provider, "stored": true })),
        Err(err) => bad_request(err),
    }
}

async fn clear_provider_session_cookie(Path(pid): Path<String>) -> Response {
    let Ok(provider) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    if !is_known_provider(provider) {
        return unknown_provider(&pid);
    }
    services::auth_session::clear_runtime_provider_cookie(&provider).await;
    ok(serde_json::json!({ "provider": provider, "stored": false }))
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
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    let search_type = query.search_type.unwrap_or_default();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(20).max(1);

    match search_type {
        SearchType::Track | SearchType::Artist => {
            match provider.search_track(&keyword, offset, limit).await {
                Ok(tracks) => ok(tracks),
                Err(err) => provider_error_response(err),
            }
        }
        SearchType::Album => match provider.search_album(&keyword, offset, limit).await {
            Ok(albums) => ok(albums),
            Err(err) => provider_error_response(err),
        },
        SearchType::Playlist => match provider.search_playlist(&keyword, offset, limit).await {
            Ok(playlists) => ok(playlists),
            Err(err) => provider_error_response(err),
        },
    }
}

async fn provider_song_url(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    let Some((track, options)) = parse_song_url_body(body) else {
        return bad_request("invalid or missing Track body");
    };
    match provider.song_url(&track, options).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_qualities(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    let Some(track) = parse_track_body(body) else {
        return bad_request("invalid or missing Track body");
    };
    match provider.track_qualities(&track).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_lyric(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    let Some(track) = parse_track_body(body) else {
        return bad_request("invalid or missing Track body");
    };
    match provider.lyric(&track).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_playlists(State(state): State<AppState>, Path(pid): Path<String>) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    match provider.playlist_list().await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_playlist_detail(
    State(state): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Query(query): Query<OffsetLimitQuery>,
) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    match provider
        .playlist_detail(&id, query.offset.unwrap_or(0), query.limit.unwrap_or(100))
        .await
    {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_albums(State(state): State<AppState>, Path(pid): Path<String>) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    match provider.album_list().await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_album_detail(
    State(state): State<AppState>,
    Path((pid, id)): Path<(String, String)>,
    Query(query): Query<OffsetLimitQuery>,
) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    match provider
        .album_detail(&id, query.offset.unwrap_or(0), query.limit.unwrap_or(100))
        .await
    {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_login_status(State(state): State<AppState>, Path(pid): Path<String>) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    match provider.login_status().await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_logout(State(state): State<AppState>, Path(pid): Path<String>) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    let had_runtime_or_env_session = services::auth_session::get_provider_cookie(&provider_id)
        .await
        .map(|cookie| !cookie.trim().is_empty())
        .unwrap_or(false);

    match provider.logout().await {
        Ok(()) => {
            services::auth_session::clear_runtime_provider_cookie(&provider_id).await;
            ok(serde_json::json!({ "provider": provider_id, "loggedOut": true }))
        }
        Err(err)
            if had_runtime_or_env_session
                && matches!(err.code, ProviderErrorCode::NotImplemented)
                && err.action.as_deref() == Some("no-session") =>
        {
            services::auth_session::clear_runtime_provider_cookie(&provider_id).await;
            ok(serde_json::json!({ "provider": provider_id, "loggedOut": true }))
        }
        Err(err) => provider_error_response(err),
    }
}

fn parse_song_url_body(body: serde_json::Value) -> Option<(Track, Option<SongUrlOptions>)> {
    if let Ok(request) = serde_json::from_value::<SongUrlTrackQualityRequest>(body.clone()) {
        return Some((
            request.track,
            Some(SongUrlOptions {
                quality: request.quality,
            }),
        ));
    }
    serde_json::from_value::<Track>(body)
        .ok()
        .map(|track| (track, None))
}

fn parse_track_body(body: serde_json::Value) -> Option<Track> {
    serde_json::from_value::<Track>(body).ok()
}

async fn provider_like(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<LikeBody>,
) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
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
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    let ids = parse_like_check_ids(query);
    if ids.is_empty() {
        return bad_request("ids required");
    }
    match provider.check_song_likes(&ids).await {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

fn parse_like_check_ids(query: LikeCheckQuery) -> Vec<String> {
    query
        .ids
        .or(query.id)
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

async fn provider_playlist_add_song(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<PlaylistAddSongBody>,
) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    match provider
        .update_song_in_playlist(&body.playlist_id, &body.track_id, true)
        .await
    {
        Ok(result) => ok(result),
        Err(err) => provider_error_response(err),
    }
}

async fn provider_playlist_del_song(
    State(state): State<AppState>,
    Path(pid): Path<String>,
    axum::Json(body): axum::Json<PlaylistAddSongBody>,
) -> Response {
    let Ok(provider_id) = pid.parse::<ProviderId>() else {
        return unknown_provider(&pid);
    };
    let Some(provider) = state.providers.get(&provider_id) else {
        return unavailable_provider(provider_id);
    };
    match provider
        .update_song_in_playlist(&body.playlist_id, &body.track_id, false)
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

fn is_known_provider(provider: ProviderId) -> bool {
    matches!(
        provider,
        ProviderId::Netease
            | ProviderId::Qq
            | ProviderId::Soda
            | ProviderId::Kugou
            | ProviderId::Spotify
    )
}

fn unknown_provider(raw: &str) -> Response {
    fail(
        StatusCode::NOT_FOUND,
        "PROVIDER_NOT_FOUND",
        format!("unknown provider: {raw}"),
    )
}

fn unavailable_provider(provider: ProviderId) -> Response {
    if !is_known_provider(provider) {
        return unknown_provider(provider.as_str());
    }
    fail(
        StatusCode::NOT_IMPLEMENTED,
        "PROVIDER_UNAVAILABLE",
        format!("provider {provider} is not wired into the registry yet"),
    )
}

fn bad_request(message: impl Into<String>) -> Response {
    fail(StatusCode::BAD_REQUEST, "BAD_REQUEST", message)
}

fn internal_error(message: impl Into<String>) -> Response {
    fail(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL", message)
}

fn provider_error_response(err: ProviderError) -> Response {
    let error_entry = json!({
        "event": "provider-error",
        "provider": err.provider,
        "code": format!("{:?}", err.code).to_uppercase(),
        "message": err.message,
        "retryable": err.retryable,
        "action": err.action,
        "rawMessage": err.raw_message,
    });
    services::diagnostics::push_recent_error(error_entry.clone());
    sidecar_log::spawn_runtime_log(error_entry);
    let status = match err.code {
        ProviderErrorCode::LoginRequired => StatusCode::UNAUTHORIZED,
        ProviderErrorCode::NotImplemented => StatusCode::NOT_IMPLEMENTED,
        ProviderErrorCode::InvalidResponse
        | ProviderErrorCode::NoResult
        | ProviderErrorCode::NoUrl
        | ProviderErrorCode::NoPlaylist => StatusCode::NOT_FOUND,
        ProviderErrorCode::Unavailable
        | ProviderErrorCode::CopyrightUnavailable
        | ProviderErrorCode::PaidRequired
        | ProviderErrorCode::TrialOnly
        | ProviderErrorCode::VipRequired => StatusCode::BAD_GATEWAY,
        ProviderErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    fail(
        status,
        format!("{:?}", err.code).to_uppercase(),
        err.message,
    )
}

fn anyhow_error_response(err: anyhow::Error) -> Response {
    match err.downcast::<ProviderError>() {
        Ok(provider_err) => provider_error_response(provider_err),
        Err(err) => {
            let entry = json!({
                "event": "internal-error",
                "message": err.to_string()
            });
            services::diagnostics::push_recent_error(entry.clone());
            sidecar_log::spawn_runtime_log(entry);
            internal_error(err.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LikeCheckQuery, is_known_provider, parse_like_check_ids};
    use crate::types::ProviderId;

    #[test]
    fn like_check_ids_accepts_the_id_alias() {
        let ids = parse_like_check_ids(LikeCheckQuery {
            ids: None,
            id: Some(" 1, ,2 ".to_owned()),
        });

        assert_eq!(ids, ["1", "2"]);
    }

    #[test]
    fn like_check_ids_prefers_ids_over_the_id_alias() {
        let ids = parse_like_check_ids(LikeCheckQuery {
            ids: Some("3".to_owned()),
            id: Some("1,2".to_owned()),
        });

        assert_eq!(ids, ["3"]);
    }

    #[test]
    fn spotify_accepts_session_cookie_routes() {
        assert!(is_known_provider(ProviderId::Spotify));
    }

    #[test]
    fn kugou_accepts_session_cookie_routes() {
        assert!(is_known_provider(ProviderId::Kugou));
    }
}
