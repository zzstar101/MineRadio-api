use std::sync::Arc;

use reqwest::Client;

use crate::{
    config::LibraryConfig,
    providers::{
        kugou::adapter::KugouAdapter,
        netease::{adapter::NeteaseAdapter, client::NeteaseClient},
        qq::adapter::QqAdapter,
        registry::ProviderRegistry,
        soda::adapter::SodaAdapter,
        spotify::{adapter::SpotifyAdapter, client::SpotifyClient},
    },
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
        soda_audio_proxy::{SodaAudioProxy, SodaAudioProxyDeps, create_soda_audio_proxy},
        soda_qr_login::{SodaQrLoginDeps, SodaQrLoginService, create_soda_qr_login_service},
        spotify_audio_proxy::{SpotifyAudioProxy, create_spotify_audio_proxy},
        weather_radio::{WeatherRadioDeps, WeatherRadioService, create_weather_radio_service},
    },
};

#[derive(Clone)]
pub(crate) struct AppServices {
    pub(crate) audio_proxy: AudioProxy,
    pub(crate) discover_requester: Arc<dyn DiscoverRequester>,
    pub(crate) image_proxy: ImageProxy,
    pub(crate) kugou_qr_login: Arc<KugouQrLoginService>,
    pub(crate) netease_qr_login: Arc<NeteaseQrLoginService>,
    pub(crate) podcast: PodcastService,
    pub(crate) qq_qr_login: Arc<QqQrLoginService>,
    pub(crate) qqmusic_qr_login: Arc<QqMusicQrLoginService>,
    pub(crate) wechat_qr_login: Arc<WechatQrLoginService>,
    pub(crate) qq_audio_proxy: QqAudioProxy,
    pub(crate) soda_audio_proxy: SodaAudioProxy,
    pub(crate) soda_qr_login: Arc<SodaQrLoginService>,
    pub(crate) spotify_audio_proxy: SpotifyAudioProxy,
    pub(crate) weather_radio: WeatherRadioService,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) config: LibraryConfig,
    pub(crate) providers: Arc<ProviderRegistry>,
    pub(crate) services: AppServices,
}

impl AppState {
    pub(crate) fn new(config: LibraryConfig) -> Self {
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
