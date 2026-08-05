use std::{future::Future, panic::AssertUnwindSafe, sync::Arc};

use futures_util::FutureExt;

use crate::{
    api::{ApiError, ApiErrorCode, ApiResult, error::from_provider_error},
    providers::error::ProviderError,
    services::cross_source_resolver::{CrossSourceResolver, ResolveSearchQuery},
    types::{ProviderId, RecommendationPage, SongUrlOptions, SongUrlResult, Track},
};

pub(crate) struct CrossSourceApi {
    resolver: Arc<CrossSourceResolver>,
}

impl CrossSourceApi {
    pub(crate) fn new(resolver: CrossSourceResolver) -> Self {
        Self {
            resolver: Arc::new(resolver),
        }
    }

    async fn call<T>(
        &self,
        operation: &'static str,
        future: impl Future<Output = anyhow::Result<T>>,
    ) -> ApiResult<T> {
        match AssertUnwindSafe(future).catch_unwind().await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => {
                if let Some(provider_error) = error.downcast_ref::<ProviderError>() {
                    return Err(from_provider_error(provider_error.clone()));
                }

                tracing::error!(operation, error = %error, "cross-source operation failed");
                Err(ApiError::new(ApiErrorCode::Internal, "internal error"))
            }
            Err(_) => {
                tracing::error!(operation, "cross-source operation panicked");
                Err(ApiError::new(ApiErrorCode::Internal, "internal error"))
            }
        }
    }

    pub(crate) async fn search_tracks(
        &self,
        keyword: &str,
        provider: Option<ProviderId>,
        limit: u32,
    ) -> ApiResult<Vec<Track>> {
        let keyword = keyword.trim();
        if keyword.is_empty() {
            return Err(ApiError::new(ApiErrorCode::BadRequest, "keyword required"));
        }

        self.call(
            "search_tracks",
            self.resolver.resolve_search(ResolveSearchQuery {
                keyword: keyword.to_owned(),
                provider,
                limit: limit.max(1),
            }),
        )
        .await
    }

    pub(crate) async fn song_url(
        &self,
        track: Track,
        options: Option<SongUrlOptions>,
    ) -> ApiResult<SongUrlResult> {
        self.call("song_url", self.resolver.resolve_song_url(track, options))
            .await
    }

    pub(crate) async fn recommendation_pages(&self) -> ApiResult<Vec<RecommendationPage>> {
        self.call(
            "recommendation_pages",
            self.resolver.resolve_recommendation_page(),
        )
        .await
    }
}
