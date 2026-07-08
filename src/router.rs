use axum::{
    Router,
    extract::State,
    http::{Method, StatusCode},
    response::Response,
    routing::get,
};
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::{
    http::response::{cors_preflight, fail, json, ok},
    providers::registry::{CapabilityMatrix, PROVIDER_IDS, build_capability_matrix},
    server::AppState,
    services,
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
