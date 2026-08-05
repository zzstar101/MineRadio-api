mod cross_source;
mod error;
mod provider;
mod qr_login;

use std::{collections::HashMap, sync::Arc};

use reqwest::Client;
use serde_json::json;

use crate::{
    config::LibraryConfig,
    providers::{
        ProviderAdapter,
        kugou::adapter::KugouAdapter,
        netease::{adapter::NeteaseAdapter, client::NeteaseClient},
        qq::adapter::QqAdapter,
        soda::adapter::SodaAdapter,
        spotify::{adapter::SpotifyAdapter, client::SpotifyClient},
    },
    services::{
        auth_session, cross_source_resolver,
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
        sidecar_log::{self, SidecarLogger, SidecarLoggerOptions},
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
    ProviderLoginQrImage, ProviderLoginQrKey, ProviderLoginStatus, RecommendationCard,
    RecommendationModule, RecommendationPage, RecommendationType, SearchType, SongLikeAck,
    SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    TrackQualityOption, VipLevel,
};

pub(crate) struct ApiInner {
    config: LibraryConfig,
    logger: SidecarLogger,
    cross_source: cross_source::CrossSourceApi,
    qq: Arc<dyn ProviderAdapter>,
    netease: Arc<dyn ProviderAdapter>,
    soda: Arc<dyn ProviderAdapter>,
    kugou: Arc<dyn ProviderAdapter>,
    spotify: Arc<dyn ProviderAdapter>,
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
    fn new(config: LibraryConfig, logger: SidecarLogger) -> Self {
        let shared_http_client = Client::new();
        let netease_client = Arc::new(NeteaseClient::with_client(shared_http_client.clone()));
        let qq_qr_client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| Client::new());
        let netease: Arc<dyn ProviderAdapter> =
            Arc::new(NeteaseAdapter::new(netease_client.clone()));
        let qq: Arc<dyn ProviderAdapter> = QqAdapter::shared();
        let soda: Arc<dyn ProviderAdapter> = SodaAdapter::shared();
        let kugou: Arc<dyn ProviderAdapter> = KugouAdapter::shared();
        let spotify_client = Arc::new(SpotifyClient::new());
        let spotify: Arc<dyn ProviderAdapter> = Arc::new(SpotifyAdapter::new(spotify_client));
        let provider_map = HashMap::from([
            (ProviderId::Netease, netease.clone()),
            (ProviderId::Qq, qq.clone()),
            (ProviderId::Soda, soda.clone()),
            (ProviderId::Kugou, kugou.clone()),
            (ProviderId::Spotify, spotify.clone()),
        ]);

        Self {
            config,
            logger,
            cross_source: cross_source::CrossSourceApi::new(
                cross_source_resolver::create_cross_source_resolver(
                    cross_source_resolver::CrossSourceResolverDeps {
                        providers: Some(provider_map),
                        provider_order: None,
                    },
                ),
            ),
            podcast: create_podcast_service_with_client(netease_client),
            weather_radio: create_weather_radio_service(WeatherRadioDeps::default()),
            qq,
            netease,
            soda,
            kugou,
            spotify,
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
        let logger = sidecar_log::create_sidecar_logger(SidecarLoggerOptions {
            file_path: config.log_path.clone(),
            max_bytes: None,
        })
        .map_err(|_| ApiError::new(ApiErrorCode::Internal, "failed to initialize logging"))?;

        let inner = Arc::new(ApiInner::new(config, logger));
        let qq_qr_login = QrLoginApi::new(inner.qq_qr_login.clone());
        let netease_qr_login = QrLoginApi::new(inner.netease_qr_login.clone());
        let soda_qr_login = QrLoginApi::new(inner.soda_qr_login.clone());
        let kugou_qr_login = QrLoginApi::new(inner.kugou_qr_login.clone());
        let qqmusic_qr_login = QrLoginApi::new(inner.qqmusic_qr_login.clone());
        let wechat_qr_login = QrLoginApi::new(inner.wechat_qr_login.clone());

        inner
            .logger
            .log(json!({
                "event": "library-startup",
                "appVersion": inner.config.app_version,
                "apiVersion": inner.config.api_version,
                "schemaVersion": inner.config.schema_version,
            }))
            .await;

        Ok(Self {
            qq: ProviderApi::new(
                inner.qq.clone(),
                vec![qq_qr_login, qqmusic_qr_login, wechat_qr_login],
            ),
            netease: ProviderApi::new(inner.netease.clone(), vec![netease_qr_login]),
            soda: ProviderApi::new(inner.soda.clone(), vec![soda_qr_login]),
            kugou: ProviderApi::new(inner.kugou.clone(), vec![kugou_qr_login]),
            spotify: ProviderApi::new(inner.spotify.clone(), Vec::new()),
            inner,
        })
    }

    pub async fn shutdown(&self) -> ApiResult<()> {
        self.inner
            .logger
            .log(json!({
                "event": "library-shutdown",
                "appVersion": self.inner.config.app_version,
                "apiVersion": self.inner.config.api_version,
                "schemaVersion": self.inner.config.schema_version,
            }))
            .await;
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

    pub async fn recommendation_pages(&self) -> ApiResult<Vec<RecommendationPage>> {
        self.inner.cross_source.recommendation_pages().await
    }

    pub fn app_version(&self) -> &str {
        &self.inner.config.app_version
    }
}
