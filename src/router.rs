use axum::{Json, Router, extract::State, routing::get};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::{http::response::ok, server::AppState, services};

pub fn build(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/providers/capabilities", get(provider_capabilities))
        .route("/diagnostics", get(diagnostics))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    Json(services::health::snapshot(&state.config))
}

async fn provider_capabilities(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    ok(state.providers.build_capability_matrix())
}

async fn diagnostics(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    ok(services::diagnostics::snapshot(&state))
}
