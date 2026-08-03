use std::sync::Arc;

use anyhow::Context;
use reqwest::Client;
use serde_json::json;
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    config::Config,
    providers::{
        kugou::adapter::KugouAdapter,
        netease::{adapter::NeteaseAdapter, client::NeteaseClient},
        qq::adapter::QqAdapter,
        registry::ProviderRegistry,
        soda::adapter::SodaAdapter,
        spotify::{adapter::SpotifyAdapter, client::SpotifyClient},
    },
    router,
    services::{
        audio_proxy::{AudioProxy, AudioProxyDeps, create_audio_proxy},
        discover_home::DiscoverRequester,
        image_proxy::{ImageProxy, ImageProxyDeps, create_image_proxy},
        kugou_qr_login::{
            KugouQrHttpApi, KugouQrLoginDeps, KugouQrLoginService, create_kugou_qr_login_service,
        },
        netease_qr_login::{NeteaseQrLoginService, create_netease_qr_login_service_with_client},
        podcast::{PodcastService, create_podcast_service_with_client},
        qq_audio_proxy::{QqAudioProxy, QqAudioProxyDeps, create_qq_audio_proxy},
        qq_qr_login_mqtt::{
            QqMusicQrLoginDeps, QqMusicQrLoginService, create_qqmusic_qr_login_service,
        },
        qq_qr_login_qq::{QqQrLoginDeps, QqQrLoginService, create_qq_qr_login_service},
        qq_qr_login_wx::{WechatQrLoginDeps, WechatQrLoginService, create_wechat_qr_login_service},
        sidecar_log,
        soda_audio_proxy::{SodaAudioProxy, SodaAudioProxyDeps, create_soda_audio_proxy},
        soda_qr_login::{SodaQrLoginDeps, SodaQrLoginService, create_soda_qr_login_service},
        spotify_audio_proxy::{SpotifyAudioProxy, create_spotify_audio_proxy},
        weather_radio::{WeatherRadioDeps, WeatherRadioService, create_weather_radio_service},
    },
};

#[derive(Clone)]
pub struct AppServices {
    pub audio_proxy: AudioProxy,
    pub discover_requester: Arc<dyn DiscoverRequester>,
    pub image_proxy: ImageProxy,
    pub kugou_qr_login: Arc<KugouQrLoginService>,
    pub netease_qr_login: Arc<NeteaseQrLoginService>,
    pub podcast: PodcastService,
    pub qq_qr_login: Arc<QqQrLoginService>,
    pub qqmusic_qr_login: Arc<QqMusicQrLoginService>,
    pub wechat_qr_login: Arc<WechatQrLoginService>,
    pub qq_audio_proxy: QqAudioProxy,
    pub soda_audio_proxy: SodaAudioProxy,
    pub soda_qr_login: Arc<SodaQrLoginService>,
    pub spotify_audio_proxy: SpotifyAudioProxy,
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
        let shared_http_client = Client::new();
        let netease_client = Arc::new(NeteaseClient::with_client(shared_http_client.clone()));
        let qq_qr_client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| Client::new());
        let mut providers = ProviderRegistry::default();
        providers.register(Arc::new(NeteaseAdapter::new(netease_client.clone())));
        providers.register(QqAdapter::shared());
        providers.register(SodaAdapter::shared());
        providers.register(KugouAdapter::shared());
        let spotify_client = Arc::new(SpotifyClient::new());
        providers.register(Arc::new(SpotifyAdapter::new(spotify_client.clone())));

        Self {
            config,
            providers: Arc::new(providers),
            services: AppServices {
                discover_requester: netease_client.clone(),
                netease_qr_login: Arc::new(create_netease_qr_login_service_with_client(
                    shared_http_client.clone(),
                )),
                podcast: create_podcast_service_with_client(netease_client),
                audio_proxy: create_audio_proxy(AudioProxyDeps::default()),
                image_proxy: create_image_proxy(ImageProxyDeps::default()),
                kugou_qr_login: Arc::new(create_kugou_qr_login_service(KugouQrLoginDeps {
                    api: Box::new(KugouQrHttpApi::with_client(shared_http_client.clone())),
                })),
                qq_qr_login: Arc::new(create_qq_qr_login_service(QqQrLoginDeps {
                    client: qq_qr_client.clone(),
                    timeout_ms: 10_000,
                })),
                qqmusic_qr_login: Arc::new(create_qqmusic_qr_login_service(QqMusicQrLoginDeps {
                    client: qq_qr_client.clone(),
                    timeout_ms: 10_000,
                })),
                wechat_qr_login: Arc::new(create_wechat_qr_login_service(WechatQrLoginDeps {
                    client: qq_qr_client,
                    timeout_ms: 10_000,
                })),
                qq_audio_proxy: create_qq_audio_proxy(QqAudioProxyDeps::default()),
                soda_audio_proxy: create_soda_audio_proxy(SodaAudioProxyDeps::default()),
                soda_qr_login: Arc::new(create_soda_qr_login_service(SodaQrLoginDeps {
                    client: shared_http_client,
                    ..SodaQrLoginDeps::default()
                })),
                spotify_audio_proxy: create_spotify_audio_proxy(spotify_client),
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
