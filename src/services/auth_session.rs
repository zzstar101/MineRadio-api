use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::types::ProviderId;

const SESSION_FILE_ENV: &str = "MINERADIO_SESSION_FILE";

static AUTH_SESSION: OnceLock<AuthSession> = OnceLock::new();

#[derive(Debug)]
pub struct AuthSession {
    runtime: RwLock<HashMap<ProviderId, String>>,
}

impl AuthSession {
    pub fn new() -> Self {
        Self {
            runtime: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get_provider_cookie(&self, provider: &str) -> Option<String> {
        self.runtime
            .read()
            .await
            .get(provider)
            .cloned()
            .or_else(|| read_persisted_cookies().remove(provider))
            .or_else(|| env_cookie(provider))
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
        set_persisted_provider_cookie(&provider, &normalized);
        Ok(())
    }

    pub async fn clear_runtime_provider_cookie(&self, provider: &str) {
        self.runtime.write().await.remove(provider);
        clear_persisted_provider_cookie(provider);
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

pub async fn clear_runtime_provider_cookie(provider: &str) {
    auth_session().clear_runtime_provider_cookie(provider).await;
}

pub async fn get_provider_cookie(provider: &str) -> Option<String> {
    auth_session().get_provider_cookie(provider).await
}

fn auth_session() -> &'static AuthSession {
    AUTH_SESSION.get_or_init(AuthSession::new)
}

fn env_cookie(provider: &str) -> Option<String> {
    let key = match provider {
        "netease" => "MINERADIO_NETEASE_COOKIE",
        "qq" => "MINERADIO_QQ_COOKIE",
        _ => "MINERADIO_SODA_COOKIE",
    };
    env::var(key)
        .ok()
        .map(|cookie| cookie.trim().to_owned())
        .filter(|cookie| !cookie.is_empty())
}

fn session_file_path() -> Option<PathBuf> {
    env::var_os(SESSION_FILE_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            env::var_os("MINERADIO_APP_DATA_DIR")
                .map(PathBuf::from)
                .filter(|path| !path.as_os_str().is_empty())
                .map(|path| path.join("provider-sessions.json"))
        })
}

fn read_persisted_cookies() -> HashMap<ProviderId, String> {
    let Some(file) = session_file_path() else {
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
            if matches!(provider.as_str(), "netease" | "qq" | "soda") && !normalized.is_empty() {
                Some((provider, normalized))
            } else {
                None
            }
        })
        .collect()
}

fn write_persisted_cookies(cookies: HashMap<ProviderId, String>) {
    let Some(file) = session_file_path() else {
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

fn set_persisted_provider_cookie(provider: &str, cookie: &str) {
    let mut cookies = read_persisted_cookies();
    cookies.insert(provider.to_owned(), cookie.to_owned());
    write_persisted_cookies(cookies);
}

fn clear_persisted_provider_cookie(provider: &str) {
    let mut cookies = read_persisted_cookies();
    cookies.remove(provider);
    write_persisted_cookies(cookies);
}

#[allow(dead_code)]
fn parse_persisted_cookies(raw: &str) -> HashMap<ProviderId, String> {
    serde_json::from_str::<PersistedProviderSessions>(raw)
        .ok()
        .and_then(|sessions| sessions.providers)
        .unwrap_or_default()
}

#[allow(dead_code)]
fn path_exists(path: &Path) -> bool {
    path.exists()
}
