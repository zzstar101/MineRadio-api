use std::{collections::HashMap, fs, path::PathBuf, sync::OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{cache, sidecar_log, types::ProviderId};

static AUTH_SESSION: OnceLock<AuthSession> = OnceLock::new();

#[derive(Debug)]
pub struct AuthSession {
    runtime: RwLock<HashMap<ProviderId, String>>,
    /// 落盘读改写的单飞闸门: 串行化持久化, 防止并发登录互相覆盖
    persist_gate: tokio::sync::Mutex<()>,
    cookie_file: Option<PathBuf>,
}

impl AuthSession {
    pub fn new() -> Self {
        Self::with_cookie_file(None)
    }

    pub fn with_cookie_file(cookie_file: Option<PathBuf>) -> Self {
        Self {
            runtime: RwLock::new(HashMap::new()),
            persist_gate: tokio::sync::Mutex::new(()),
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
        {
            // 单飞闸门
            let _guard = self.persist_gate.lock().await;
            self.set_persisted_provider_cookie(&provider, &normalized);
        }
        cache::remove(provider, "recommendation_page").await;
        Ok(())
    }

    pub async fn clear_runtime_provider_cookie(&self, provider: &ProviderId) {
        self.runtime.write().await.remove(provider);
        {
            let _guard = self.persist_gate.lock().await;
            self.clear_persisted_provider_cookie(provider);
        }
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
            let tmp = file.with_extension("tmp");
            if let Err(err) = fs::write(&tmp, json).and_then(|()| fs::rename(&tmp, file)) {
                sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                    "auth session 写盘失败: {err}"
                )));
            }
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_cookie_file(tag: &str) -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "mineradio_auth_test_{}_{}_{}.json",
            std::process::id(),
            tag,
            n
        ))
    }

    /// 并发设置两个 provider 的 Cookie, 落盘不得互相覆盖
    #[tokio::test]
    async fn concurrent_provider_cookies_both_persisted() {
        let file = temp_cookie_file("concurrent");
        let session = AuthSession::with_cookie_file(Some(file.clone()));

        let (r1, r2) = tokio::join!(
            session.set_runtime_provider_cookie(ProviderId::Netease, "netease=1".to_owned()),
            session.set_runtime_provider_cookie(ProviderId::Qq, "qq=2".to_owned()),
        );
        r1.expect("set netease");
        r2.expect("set qq");

        let persisted = session.read_persisted_cookies();
        assert_eq!(persisted.get(&ProviderId::Netease).unwrap(), "netease=1");
        assert_eq!(persisted.get(&ProviderId::Qq).unwrap(), "qq=2");

        let _ = std::fs::remove_file(&file);
    }

    /// 登出只摘除自己的条目; 落盘始终是可解析的完整 JSON
    #[tokio::test]
    async fn persisted_file_stays_complete_across_clear() {
        let file = temp_cookie_file("clear");
        let session = AuthSession::with_cookie_file(Some(file.clone()));

        session
            .set_runtime_provider_cookie(ProviderId::Netease, "a=1".to_owned())
            .await
            .unwrap();
        session
            .set_runtime_provider_cookie(ProviderId::Soda, "b=2".to_owned())
            .await
            .unwrap();
        session
            .clear_runtime_provider_cookie(&ProviderId::Netease)
            .await;

        let persisted = session.read_persisted_cookies();
        assert!(persisted.get(&ProviderId::Netease).is_none());
        assert_eq!(persisted.get(&ProviderId::Soda).unwrap(), "b=2");

        let raw = std::fs::read_to_string(&file).unwrap();
        serde_json::from_str::<PersistedProviderSessions>(&raw).expect("valid json");

        let _ = std::fs::remove_file(&file);
    }
}
