mod cross_source;
mod error;
mod provider;
mod qr_login;

use std::sync::Arc;

use crate::{
    config::LibraryConfig,
    providers::{ProviderAdapter, registry::ProviderRegistry},
    server::AppState,
    services::{auth_session, cross_source_resolver, sidecar_log},
};
use serde_json::json;

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
    pub(crate) state: AppState,
    cross_source: cross_source::CrossSourceApi,
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

        let state = AppState::new(config.into());
        let qq = provider_adapter(&state.providers, ProviderId::Qq)?;
        let netease = provider_adapter(&state.providers, ProviderId::Netease)?;
        let soda = provider_adapter(&state.providers, ProviderId::Soda)?;
        let kugou = provider_adapter(&state.providers, ProviderId::Kugou)?;
        let spotify = provider_adapter(&state.providers, ProviderId::Spotify)?;
        let qq_qr_login = QrLoginApi::new(state.services.qq_qr_login.clone());
        let netease_qr_login = QrLoginApi::new(state.services.netease_qr_login.clone());
        let soda_qr_login = QrLoginApi::new(state.services.soda_qr_login.clone());
        let kugou_qr_login = QrLoginApi::new(state.services.kugou_qr_login.clone());
        let qqmusic_qr_login = QrLoginApi::new(state.services.qqmusic_qr_login.clone());
        let wechat_qr_login = QrLoginApi::new(state.services.wechat_qr_login.clone());
        let cross_source = cross_source::CrossSourceApi::new(
            cross_source_resolver::create_cross_source_resolver(
                cross_source_resolver::CrossSourceResolverDeps {
                    providers: Some(state.providers.all()),
                    provider_order: None,
                },
            ),
        );
        let inner = Arc::new(ApiInner {
            state,
            cross_source,
        });

        sidecar_log::spawn_runtime_log(json!({
            "event": "library-startup",
            "appVersion": inner.state.config.app_version,
            "apiVersion": inner.state.config.api_version,
            "schemaVersion": inner.state.config.schema_version,
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
        &self.inner.state.config.app_version
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
