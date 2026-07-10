use std::sync::Arc;

use anyhow::Context;
use serde_json::json;
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    config::Config,
    providers::{
        netease::{adapter::NeteaseAdapter, client::NeteaseClient},
        qq::adapter::QqAdapter,
        registry::ProviderRegistry,
        soda::adapter::SodaAdapter,
    },
    router,
    services::{
        audio_proxy::{AudioProxy, AudioProxyDeps, create_audio_proxy},
        discover_home::DiscoverRequester,
        image_proxy::{ImageProxy, ImageProxyDeps, create_image_proxy},
        netease_qr_login::{NeteaseQrLoginService, create_netease_qr_login_service_with_client},
        podcast::{PodcastService, create_podcast_service_with_client},
        qq_qr_login::{QqQrLoginDeps, QqQrLoginService, create_qq_qr_login_service},
        sidecar_log,
        soda_audio_proxy::{SodaAudioProxy, SodaAudioProxyDeps, create_soda_audio_proxy},
        soda_qr_login::{SodaQrLoginDeps, SodaQrLoginService, create_soda_qr_login_service},
        weather_radio::{WeatherRadioDeps, WeatherRadioService, create_weather_radio_service},
    },
};

#[derive(Clone)]
pub struct AppServices {
    pub audio_proxy: AudioProxy,
    pub discover_requester: Arc<dyn DiscoverRequester>,
    pub image_proxy: ImageProxy,
    pub netease_qr_login: Arc<NeteaseQrLoginService>,
    pub podcast: PodcastService,
    pub qq_qr_login: Arc<QqQrLoginService>,
    pub soda_audio_proxy: SodaAudioProxy,
    pub soda_qr_login: Arc<SodaQrLoginService>,
    pub weather_radio: WeatherRadioService,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub providers: Arc<ProviderRegistry>,
    pub services: AppServices,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let netease_client = Arc::new(NeteaseClient::new());
        let mut providers = ProviderRegistry::default();
        providers.register(Arc::new(NeteaseAdapter::new(netease_client.clone())));
        providers.register(QqAdapter::shared());
        providers.register(SodaAdapter::shared());

        Self {
            config,
            providers: Arc::new(providers),
            services: AppServices {
                discover_requester: netease_client.clone(),
                netease_qr_login: Arc::new(create_netease_qr_login_service_with_client(
                    netease_client.clone(),
                )),
                podcast: create_podcast_service_with_client(netease_client),
                audio_proxy: create_audio_proxy(AudioProxyDeps::default()),
                image_proxy: create_image_proxy(ImageProxyDeps::default()),
                qq_qr_login: Arc::new(create_qq_qr_login_service(QqQrLoginDeps::default())),
                soda_audio_proxy: create_soda_audio_proxy(SodaAudioProxyDeps::default()),
                soda_qr_login: Arc::new(create_soda_qr_login_service(SodaQrLoginDeps::default())),
                weather_radio: create_weather_radio_service(WeatherRadioDeps::default()),
            },
        }
    }
}

pub async fn serve(config: Config) -> anyhow::Result<()> {
    let listener = TcpListener::bind(config.bind_addr())
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr()))?;
    let local_addr = listener.local_addr()?;
    let state = AppState::new(config);
    let app_version = state.config.app_version.clone();
    let api_version = state.config.api_version.clone();
    let schema_version = state.config.schema_version.clone();
    let app = router::build(state);

    info!(%local_addr, "MineRadio API sidecar listening");
    sidecar_log::spawn_runtime_log(json!({
        "event": "startup",
        "localAddr": local_addr.to_string(),
        "appVersion": app_version,
        "apiVersion": api_version,
        "schemaVersion": schema_version
    }));

    axum::serve(listener, app)
        .await
        .context("MineRadio API server stopped unexpectedly")
}
