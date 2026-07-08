use std::{sync::Arc, time::SystemTime};

use anyhow::Context;
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    config::Config,
    providers::registry::ProviderRegistry,
    router,
    services::{
        audio_proxy::{AudioProxy, AudioProxyDeps, create_audio_proxy},
        image_proxy::{ImageProxy, ImageProxyDeps, create_image_proxy},
        qq_qr_login::{QqQrLoginDeps, QqQrLoginService, create_qq_qr_login_service},
        soda_audio_proxy::{SodaAudioProxy, SodaAudioProxyDeps, create_soda_audio_proxy},
        soda_qr_login::{SodaQrLoginDeps, SodaQrLoginService, create_soda_qr_login_service},
        weather_radio::{WeatherRadioDeps, WeatherRadioService, create_weather_radio_service},
    },
};

#[derive(Clone)]
pub struct AppServices {
    pub audio_proxy: AudioProxy,
    pub image_proxy: ImageProxy,
    pub qq_qr_login: Arc<QqQrLoginService>,
    pub soda_audio_proxy: SodaAudioProxy,
    pub soda_qr_login: Arc<SodaQrLoginService>,
    pub weather_radio: WeatherRadioService,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub providers: Arc<ProviderRegistry>,
    pub started_at: SystemTime,
    pub services: AppServices,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            providers: Arc::new(ProviderRegistry::default()),
            started_at: SystemTime::now(),
            services: AppServices {
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
    let app = router::build(state);

    info!(%local_addr, "MineRadio API sidecar listening");

    axum::serve(listener, app)
        .await
        .context("MineRadio API server stopped unexpectedly")
}
