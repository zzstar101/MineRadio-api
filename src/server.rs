use std::{sync::Arc, time::SystemTime};

use anyhow::Context;
use tokio::net::TcpListener;
use tracing::info;

use crate::{config::Config, providers::registry::ProviderRegistry, router};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub providers: Arc<ProviderRegistry>,
    pub started_at: SystemTime,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            providers: Arc::new(ProviderRegistry::default()),
            started_at: SystemTime::now(),
        }
    }
}

pub async fn serve(config: Config) -> anyhow::Result<()> {
    let listener = TcpListener::bind(config.bind_addr())
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr()))?;
    let local_addr = listener.local_addr()?;
    let state = AppState::new(config);
    let app = router::build(state);

    info!(%local_addr, "MineRadio API sidecar listening");

    axum::serve(listener, app)
        .await
        .context("MineRadio API server stopped unexpectedly")
}
