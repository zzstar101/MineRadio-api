mod cross_source;
mod error;
mod provider;
mod qr_login;

use std::sync::Arc;

use reqwest::Client;
use serde_json::json;

use crate::{
    config::LibraryConfig,
    providers::{
        ProviderAdapter,
        kugou::adapter::KugouAdapter,
        netease::{adapter::NeteaseAdapter, client::NeteaseClient},
        qq::adapter::QqAdapter,
        registry::ProviderRegistry,
        soda::adapter::SodaAdapter,
        spotify::{adapter::SpotifyAdapter, client::SpotifyClient},
    },
    services::{
        auth_session,
        cross_source_resolver,
        discover_home::DiscoverRequester,
        kugou_qr_login::{
            KugouQrHttpApi, KugouQrLoginDeps, KugouQrLoginService, create_kugou_qr_login_service,
        },
        netease_qr_login::{NeteaseQrLoginService, create_netease_qr_login_service_with_client},
        podcast::{PodcastService, create_podcast_service_with_client},
        qq_qr_login_mqtt::{
            QqMusicQrLoginDeps, QqMusicQrLoginService, create_qqmusic_qr_login_service,
        },
        qq_qr_login_qq::{QqQrLoginDeps, QqQrLoginService, create_qq_qr_login_service},
        qq_qr_login_wx::{WechatQrLoginDeps, WechatQrLoginService, create_wechat_qr_login_service},
        sidecar_log,
        soda_qr_login::{SodaQrLoginDeps, SodaQrLoginService, create_soda_qr_login_service},
        weather_radio::{WeatherRadioDeps, WeatherRadioService, create_weather_radio_service},
    },
};

pub use error::{ApiError, ApiErrorCode, ApiResult};
pub use provider::ProviderApi;
pub use qr_login::QrLoginApi;

pub use crate::types::{
    AlbumDetail, AlbumSummary, LyricLine, LyricPayload, LyricWord, PlayableState,
    PlaylistAddSongAck, PlaylistDetail, PlaylistSummary, ProviderId, ProviderLoginQrCheck,
    ProviderLoginQrImage, ProviderLoginQrKey, ProviderLoginStatus, SearchType, SongLikeAck,
    SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    TrackQualityOption, VipLevel,
};

pub(crate) struct ApiInner {
    config: LibraryConfig,
    providers: Arc<ProviderRegistry>,
    cross_source: cross_source::CrossSourceApi,
    discover_requester: Arc<dyn DiscoverRequester>,
    podcast: PodcastService,
    weather_radio: WeatherRadioService,
    qq_qr_login: Arc<QqQrLoginService>,
    qqmusic_qr_login: Arc<QqMusicQrLoginService>,
    wechat_qr_login: Arc<WechatQrLoginService>,
    netease_qr_login: Arc<NeteaseQrLoginService>,
    soda_qr_login: Arc<SodaQrLoginService>,
    kugou_qr_login: Arc<KugouQrLoginService>,
}

impl ApiInner {
    fn new(config: LibraryConfig) -> Self {
        let shared_http_client = Client::new();
        let netease_client = Arc::new(NeteaseClient::with_client(shared_http_client.clone()));
        let qq_qr_client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| Client::new());
        let mut registry = ProviderRegistry::default();
        registry.register(Arc::new(NeteaseAdapter::new(netease_client.clone())));
        registry.register(QqAdapter::shared());
        registry.register(SodaAdapter::shared());
        registry.register(KugouAdapter::shared());
        let spotify_client = Arc::new(SpotifyClient::new());
        registry.register(Arc::new(SpotifyAdapter::new(spotify_client)));
        let providers = Arc::new(registry);

        Self {
            config,
            cross_source: cross_source::CrossSourceApi::new(
                cross_source_resolver::create_cross_source_resolver(
                    cross_source_resolver::CrossSourceResolverDeps {
                        providers: Some(providers.all()),
                        provider_order: None,
                    },
                ),
            ),
            discover_requester: netease_client.clone(),
            podcast: create_podcast_service_with_client(netease_client),
            weather_radio: create_weather_radio_service(WeatherRadioDeps::default()),
            qq_qr_login: Arc::new(create_qq_qr_login_service(QqQrLoginDeps {
                client: qq_qr_client.clone(),
                timeout_ms: 15_000,
            })),
            qqmusic_qr_login: Arc::new(create_qqmusic_qr_login_service(QqMusicQrLoginDeps {
                client: qq_qr_client.clone(),
                timeout_ms: 10_000,
            })),
            wechat_qr_login: Arc::new(create_wechat_qr_login_service(WechatQrLoginDeps {
                client: qq_qr_client,
                timeout_ms: 10_000,
            })),
            netease_qr_login: Arc::new(create_netease_qr_login_service_with_client(
                shared_http_client.clone(),
            )),
            soda_qr_login: Arc::new(create_soda_qr_login_service(SodaQrLoginDeps {
                client: shared_http_client.clone(),
                ..SodaQrLoginDeps::default()
            })),
            kugou_qr_login: Arc::new(create_kugou_qr_login_service(KugouQrLoginDeps {
                api: Box::new(KugouQrHttpApi::with_client(shared_http_client.clone())),
            })),
            providers,
        }
    }
}

/// The public MineRadio API facade and lifecycle owner.
#[derive(Clone)]
pub struct Api {
    inner: Arc<ApiInner>,
    pub qq: ProviderApi,
    pub netease: ProviderApi,
    pub soda: ProviderApi,
    pub kugou: ProviderApi,
    pub spotify: ProviderApi,
}

impl Api {
    pub async fn init(config: LibraryConfig) -> ApiResult<Self> {
        auth_session::configure(config.cookie_file.clone())
            .map_err(|message| ApiError::new(ApiErrorCode::Internal, message))?;
        sidecar_log::configure_library_logger(config.log_path.as_deref())
            .map_err(|_| ApiError::new(ApiErrorCode::Internal, "failed to initialize logging"))?;

        let inner = Arc::new(ApiInner::new(config));
        let qq = provider_adapter(&inner.providers, ProviderId::Qq)?;
        let netease = provider_adapter(&inner.providers, ProviderId::Netease)?;
        let soda = provider_adapter(&inner.providers, ProviderId::Soda)?;
        let kugou = provider_adapter(&inner.providers, ProviderId::Kugou)?;
        let spotify = provider_adapter(&inner.providers, ProviderId::Spotify)?;
        let qq_qr_login = QrLoginApi::new(inner.qq_qr_login.clone());
        let netease_qr_login = QrLoginApi::new(inner.netease_qr_login.clone());
        let soda_qr_login = QrLoginApi::new(inner.soda_qr_login.clone());
        let kugou_qr_login = QrLoginApi::new(inner.kugou_qr_login.clone());
        let qqmusic_qr_login = QrLoginApi::new(inner.qqmusic_qr_login.clone());
        let wechat_qr_login = QrLoginApi::new(inner.wechat_qr_login.clone());

        sidecar_log::spawn_runtime_log(json!({
            "event": "library-startup",
            "appVersion": inner.config.app_version,
            "apiVersion": inner.config.api_version,
            "schemaVersion": inner.config.schema_version,
        }));

        Ok(Self {
            qq: ProviderApi::new(
                qq,
                vec![qq_qr_login, qqmusic_qr_login, wechat_qr_login],
            ),
            netease: ProviderApi::new(netease, vec![netease_qr_login]),
            soda: ProviderApi::new(soda, vec![soda_qr_login]),
            kugou: ProviderApi::new(kugou, vec![kugou_qr_login]),
            spotify: ProviderApi::new(spotify, Vec::new()),
            inner,
        })
    }

    pub async fn shutdown(&self) -> ApiResult<()> {
        Ok(())
    }

    pub async fn search_tracks(
        &self,
        keyword: &str,
        provider: Option<ProviderId>,
        limit: u32,
    ) -> ApiResult<Vec<Track>> {
        self.inner
            .cross_source
            .search_tracks(keyword, provider, limit)
            .await
    }

    pub async fn song_url(
        &self,
        track: Track,
        options: Option<SongUrlOptions>,
    ) -> ApiResult<SongUrlResult> {
        self.inner.cross_source.song_url(track, options).await
    }

    pub fn app_version(&self) -> &str {
        &self.inner.config.app_version
    }
}

fn provider_adapter(
    registry: &ProviderRegistry,
    provider: ProviderId,
) -> ApiResult<Arc<dyn ProviderAdapter>> {
    registry.get(&provider).ok_or_else(|| {
        ApiError::new(
            ApiErrorCode::Internal,
            format!("provider {provider} was not initialized"),
        )
    })
}
