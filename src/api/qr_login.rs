use std::{future::Future, panic::AssertUnwindSafe, sync::Arc};

use futures_util::FutureExt;

use crate::{
    api::{ApiError, ApiErrorCode, ApiResult},
    services::qr_login::QrLogin,
    types::{ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey},
};

/// Public QR login facade for a provider login protocol.
#[derive(Clone)]
pub struct QrLoginApi {
    service: Arc<dyn QrLogin>,
}

impl QrLoginApi {
    pub(crate) fn new(service: Arc<dyn QrLogin>) -> Self {
        Self { service }
    }

    async fn call<T>(
        &self,
        operation: &'static str,
        failure: ApiErrorCode,
        future: impl Future<Output = anyhow::Result<T>>,
    ) -> ApiResult<T> {
        match AssertUnwindSafe(future).catch_unwind().await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) if error.to_string() == "KUGOU_QR_LOGIN_NOT_IMPLEMENTED" => Err(
                ApiError::new(
                    ApiErrorCode::NotImplemented,
                    "QR login is not implemented for this provider",
                ),
            ),
            Ok(Err(error)) => {
                tracing::warn!(operation, error = %error, "QR login operation failed");
                Err(ApiError::new(failure, "QR login request failed"))
            }
            Err(_) => {
                tracing::error!(operation, "QR login operation panicked");
                Err(ApiError::new(ApiErrorCode::Internal, "internal error"))
            }
        }
    }

    pub async fn create_key(&self) -> ApiResult<ProviderLoginQrKey> {
        self.call("create_key", ApiErrorCode::Internal, self.service.create_key())
            .await
    }

    pub async fn create_image(&self, key: &str) -> ApiResult<ProviderLoginQrImage> {
        self.call(
            "create_image",
            ApiErrorCode::BadRequest,
            self.service.create_image(key),
        )
        .await
    }

    pub async fn check(&self, key: &str) -> ApiResult<ProviderLoginQrCheck> {
        self.call("check", ApiErrorCode::BadRequest, self.service.check(key))
            .await
    }
}
