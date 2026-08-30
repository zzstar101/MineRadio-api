use std::{collections::HashMap, sync::Arc};

use reqwest::Client;
use serde_json::json;

use crate::{
    auth_session, cache,
    config::LibraryConfig,
    cross_source,
    podcast::{PodcastService, create_podcast_service_with_client},
    providers::{
        ProviderAdapter,
        kugou::adapter::KugouAdapter,
        netease::{adapter::NeteaseAdapter, client::NeteaseClient},
        qq::adapter::QqAdapter,
        soda::adapter::SodaAdapter,
        spotify::{adapter::SpotifyAdapter, client::SpotifyClient},
    },
    qr_login::{
        QrLoginKind,
        kugou::{KugouQrHttpApi, KugouQrLoginDeps, create_kugou_qr_login_service},
        netease::create_netease_qr_login_service_with_client,
        qq::{QqQrLoginDeps, create_qq_qr_login_service},
        qq_music::{QqMusicQrLoginDeps, create_qqmusic_qr_login_service},
        soda::{SodaQrLoginDeps, create_soda_qr_login_service},
        wechat::{WechatQrLoginDeps, create_wechat_qr_login_service},
    },
    sidecar_log::{self, SidecarLogger},
    weather_radio::{WeatherRadioDeps, WeatherRadioService, create_weather_radio_service},
};

pub use crate::error::{ApiError, ApiErrorCode, ApiResult};
pub use crate::provider::ProviderApi;
pub use crate::qr_login_api::QrLoginApi;

pub use crate::types::{
    AlbumDetail, AlbumSummary, LyricLine, LyricPayload, LyricWord, PlayableState,
    PlaylistAddSongAck, PlaylistDetail, PlaylistSummary, ProviderId, ProviderLoginQrCheck,
    ProviderLoginQrImage, ProviderLoginQrKey, ProviderLoginStatus, RecommendationCard,
    RecommendationCardKind, RecommendationModule, RecommendationPage, SearchType, SongLikeAck,
    SongLikeCheckAck, SongUrlOptions, SongUrlResult, Track, TrackQualityAvailability,
    TrackQualityOption, VipLevel,
};

pub(crate) struct ApiInner {
    config: LibraryConfig,
    logger: Arc<SidecarLogger>,
    cross_source: cross_source::CrossSourceApi,
    qq: Arc<dyn ProviderAdapter>,
    netease: Arc<dyn ProviderAdapter>,
    soda: Arc<dyn ProviderAdapter>,
    kugou: Arc<dyn ProviderAdapter>,
    spotify: Arc<dyn ProviderAdapter>,
    podcast: PodcastService,
    weather_radio: WeatherRadioService,
    qr_logins: HashMap<QrLoginKind, QrLoginApi>,
}

impl ApiInner {
    fn new(config: LibraryConfig, logger: Arc<SidecarLogger>) -> Self {
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
                cross_source::create_cross_source_resolver(cross_source::CrossSourceResolverDeps {
                    providers: provider_map,
                }),
            ),
            podcast: create_podcast_service_with_client(netease_client),
            weather_radio: create_weather_radio_service(WeatherRadioDeps::default()),
            qq,
            netease,
            soda,
            kugou,
            spotify,
            qr_logins: HashMap::from([
                (
                    QrLoginKind::Qq,
                    QrLoginApi::new(
                        QrLoginKind::Qq,
                        Arc::new(create_qq_qr_login_service(QqQrLoginDeps {
                            client: qq_qr_client.clone(),
                            timeout_ms: 15_000,
                        })),
                    ),
                ),
                (
                    QrLoginKind::QqMusic,
                    QrLoginApi::new(
                        QrLoginKind::QqMusic,
                        Arc::new(create_qqmusic_qr_login_service(QqMusicQrLoginDeps {
                            client: qq_qr_client.clone(),
                        })),
                    ),
                ),
                (
                    QrLoginKind::Wechat,
                    QrLoginApi::new(
                        QrLoginKind::Wechat,
                        Arc::new(create_wechat_qr_login_service(WechatQrLoginDeps {
                            client: qq_qr_client,
                            timeout_ms: 10_000,
                        })),
                    ),
                ),
                (
                    QrLoginKind::Netease,
                    QrLoginApi::new(
                        QrLoginKind::Netease,
                        Arc::new(create_netease_qr_login_service_with_client(
                            shared_http_client.clone(),
                        )),
                    ),
                ),
                (
                    QrLoginKind::Soda,
                    QrLoginApi::new(
                        QrLoginKind::Soda,
                        Arc::new(create_soda_qr_login_service(SodaQrLoginDeps {
                            client: shared_http_client.clone(),
                            ..SodaQrLoginDeps::default()
                        })),
                    ),
                ),
                (
                    QrLoginKind::Kugou,
                    QrLoginApi::new(
                        QrLoginKind::Kugou,
                        Arc::new(create_kugou_qr_login_service(KugouQrLoginDeps {
                            api: Box::new(KugouQrHttpApi::with_client(shared_http_client.clone())),
                        })),
                    ),
                ),
            ]),
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
        let data_dir = config
            .persistent_data_dir()
            .map_err(|message| ApiError::new(ApiErrorCode::Internal, message))?;
        cache::configure(data_dir.clone())
            .map_err(|message| ApiError::new(ApiErrorCode::Internal, message))?;
        auth_session::configure(Some(data_dir.join("provider-sessions.json")))
            .map_err(|message| ApiError::new(ApiErrorCode::Internal, message))?;
        let logger = sidecar_log::configure_runtime_logger(Some(&data_dir.join("logs")))
            .map_err(|_| ApiError::new(ApiErrorCode::Internal, "failed to initialize logging"))?;

        if let Err(e) = crate::utils::cryptors::csigner::init() {
            logger
                .log(serde_json::json!(format!("csigner init error: {e}")))
                .await
        }

        let inner = Arc::new(ApiInner::new(config, logger));
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
            qq: ProviderApi::new(inner.qq.clone()),
            netease: ProviderApi::new(inner.netease.clone()),
            soda: ProviderApi::new(inner.soda.clone()),
            kugou: ProviderApi::new(inner.kugou.clone()),
            spotify: ProviderApi::new(inner.spotify.clone()),
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

    pub async fn search_albums(
        &self,
        keyword: &str,
        provider: Option<ProviderId>,
        limit: u32,
    ) -> ApiResult<Vec<AlbumSummary>> {
        self.inner
            .cross_source
            .search_albums(keyword, provider, limit)
            .await
    }

    pub async fn search_playlists(
        &self,
        keyword: &str,
        provider: Option<ProviderId>,
        limit: u32,
    ) -> ApiResult<Vec<PlaylistSummary>> {
        self.inner
            .cross_source
            .search_playlists(keyword, provider, limit)
            .await
    }

    pub async fn song_url(
        &self,
        track: Track,
        options: Option<SongUrlOptions>,
    ) -> ApiResult<SongUrlResult> {
        self.inner.cross_source.song_url(track, options).await
    }

    pub async fn recommendation_pages(&self, refresh: bool) -> ApiResult<Vec<RecommendationPage>> {
        self.inner.cross_source.recommendation_pages(refresh).await
    }

    pub fn qr_login(&self, kind: QrLoginKind) -> Option<&QrLoginApi> {
        self.inner.qr_logins.get(&kind)
    }

    pub fn qr_login_kinds(&self) -> impl Iterator<Item = QrLoginKind> + '_ {
        self.inner.qr_logins.keys().copied()
    }

    pub fn app_version(&self) -> &str {
        &self.inner.config.app_version
    }
}
