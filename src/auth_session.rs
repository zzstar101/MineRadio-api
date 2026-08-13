use std::{collections::HashMap, fs, path::PathBuf, sync::OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{cache, types::ProviderId};

static AUTH_SESSION: OnceLock<AuthSession> = OnceLock::new();

#[derive(Debug)]
pub struct AuthSession {
    runtime: RwLock<HashMap<ProviderId, String>>,
    cookie_file: Option<PathBuf>,
}

impl AuthSession {
    pub fn new() -> Self {
        Self::with_cookie_file(None)
    }

    pub fn with_cookie_file(cookie_file: Option<PathBuf>) -> Self {
        Self {
            runtime: RwLock::new(HashMap::new()),
            cookie_file,
        }
    }

    pub async fn get_provider_cookie(&self, provider: &ProviderId) -> Option<String> {
        self.runtime
            .read()
            .await
            .get(provider)
            .cloned()
            .or_else(|| self.read_persisted_cookies().remove(provider))
    }

    pub async fn set_runtime_provider_cookie(
        &self,
        provider: ProviderId,
        cookie: String,
    ) -> Result<(), String> {
        let normalized = cookie.trim().to_owned();
        if normalized.is_empty() {
            return Err("EMPTY_COOKIE".to_owned());
        }
        self.runtime
            .write()
            .await
            .insert(provider.clone(), normalized.clone());
        self.set_persisted_provider_cookie(&provider, &normalized);
        cache::remove(provider, "recommendation_page").await;
        Ok(())
    }

    pub async fn clear_runtime_provider_cookie(&self, provider: &ProviderId) {
        self.runtime.write().await.remove(provider);
        self.clear_persisted_provider_cookie(provider);
        cache::remove(*provider, "recommendation_page").await;
    }
}

impl Default for AuthSession {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct PersistedProviderSessions {
    version: Option<u8>,
    providers: Option<HashMap<ProviderId, String>>,
}

pub async fn set_runtime_provider_cookie(
    provider: ProviderId,
    cookie: String,
) -> Result<(), String> {
    auth_session()
        .set_runtime_provider_cookie(provider, cookie)
        .await
}

pub async fn clear_runtime_provider_cookie(provider: &ProviderId) {
    auth_session().clear_runtime_provider_cookie(provider).await;
}

pub async fn get_provider_cookie(provider: &ProviderId) -> Option<String> {
    auth_session().get_provider_cookie(provider).await
}

pub fn configure(cookie_file: Option<PathBuf>) -> Result<(), &'static str> {
    if let Some(existing) = AUTH_SESSION.get() {
        return if existing.cookie_file == cookie_file {
            Ok(())
        } else {
            Err("provider cookie storage has already been initialized with a different path")
        };
    }

    let session = AuthSession::with_cookie_file(cookie_file);
    AUTH_SESSION
        .set(session)
        .map_err(|_| "provider cookie storage has already been initialized")
}

fn auth_session() -> &'static AuthSession {
    AUTH_SESSION.get_or_init(AuthSession::new)
}

impl AuthSession {
    fn read_persisted_cookies(&self) -> HashMap<ProviderId, String> {
        let Some(file) = &self.cookie_file else {
            return HashMap::new();
        };
        let Ok(raw) = fs::read_to_string(file) else {
            return HashMap::new();
        };
        let Ok(parsed) = serde_json::from_str::<PersistedProviderSessions>(&raw) else {
            return HashMap::new();
        };

        parsed
            .providers
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(provider, cookie)| {
                let normalized = cookie.trim().to_owned();
                if matches!(
                    provider.as_str(),
                    "netease" | "qq" | "soda" | "kugou" | "spotify"
                ) && !normalized.is_empty()
                {
                    Some((provider, normalized))
                } else {
                    None
                }
            })
            .collect()
    }

    fn write_persisted_cookies(&self, cookies: HashMap<ProviderId, String>) {
        let Some(file) = &self.cookie_file else {
            return;
        };
        if let Some(parent) = file.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let body = PersistedProviderSessions {
            version: Some(1),
            providers: Some(cookies),
        };
        if let Ok(json) = serde_json::to_string_pretty(&body) {
            let _ = fs::write(file, json);
        }
    }

    fn set_persisted_provider_cookie(&self, provider: &ProviderId, cookie: &str) {
        let mut cookies = self.read_persisted_cookies();
        cookies.insert(*provider, cookie.to_owned());
        self.write_persisted_cookies(cookies);
    }

    fn clear_persisted_provider_cookie(&self, provider: &ProviderId) {
        let mut cookies = self.read_persisted_cookies();
        cookies.remove(provider);
        self.write_persisted_cookies(cookies);
    }
}
