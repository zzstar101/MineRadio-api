use axum::{
    Router,
    extract::State,
    http::{Method, StatusCode},
    response::Response,
    routing::get,
};
use tower_http::trace::TraceLayer;

use crate::{
    http::response::{cors_preflight, fail, json, ok},
    server::AppState,
    services,
};

pub fn build(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health).options(preflight))
        .route(
            "/providers/capabilities",
            get(provider_capabilities).options(preflight),
        )
        .route("/diagnostics", get(diagnostics).options(preflight))
        .fallback(fallback)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    json(services::health::snapshot(&state.config), StatusCode::OK)
}

async fn provider_capabilities(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    ok(state.providers.build_capability_matrix())
}

async fn diagnostics(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    ok(services::diagnostics::snapshot(&state))
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
