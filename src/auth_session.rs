use std::{collections::HashMap, fs, path::PathBuf, sync::OnceLock};

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
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
            .insert(provider, normalized.clone());
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
    /// v1 明文遗留字段, 仅用于读取旧文件后升级
    providers: Option<HashMap<ProviderId, String>>,
    /// v2: base64(nonce || AES-256-GCM(providers 序列化结果))
    data: Option<String>,
}

/// 兜底密钥: 仅当系统凭据库不可用(常见于无桌面环境的 Linux)时使用
const FALLBACK_COOKIE_KEY: [u8; 32] = [
    0x9f, 0x27, 0x51, 0xc4, 0x83, 0xae, 0x0d, 0x66, 0x1b, 0x74, 0xe2, 0x98, 0xf5, 0x3a, 0xcc, 0x10,
    0x6d, 0xb8, 0x24, 0x47, 0xdf, 0x09, 0x71, 0xbe, 0x38, 0xa5, 0xec, 0x52, 0x17, 0x80, 0x43, 0xda,
];

/// 解析后的进程级密钥缓存
static COOKIE_KEY: OnceLock<[u8; 32]> = OnceLock::new();

const KEY_SERVICE: &str = "Mineradio-Tauri-api";
const KEY_USER: &str = "cookie-aes-key";

/// 从系统凭据库取密钥(Windows 凭据管理器 / macOS 钥匙串 / Secret Service);
/// 不存在则生成新密钥写回; 凭据库整体不可用时回退内置密钥并告警
fn resolve_cookie_key() -> [u8; 32] {
    *COOKIE_KEY.get_or_init(provision_cookie_key)
}

fn provision_cookie_key() -> [u8; 32] {
    let fallback = || {
        sidecar_log::spawn_runtime_log(serde_json::json!(
            "auth session: 系统凭据库不可用, Cookie 加密回退到内置过渡密钥"
        ));
        FALLBACK_COOKIE_KEY
    };

    let Ok(entry) = keyring::Entry::new(KEY_SERVICE, KEY_USER) else {
        return fallback();
    };
    match entry.get_password() {
        Ok(stored) => decode_key(&stored).unwrap_or_else(fallback),
        Err(keyring::Error::NoEntry) => {
            let key = rand::random::<[u8; 32]>();
            if let Err(err) = entry.set_password(&encode_key(&key)) {
                sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                    "auth session: 密钥写入凭据库失败({err}), 回退内置过渡密钥"
                )));
                return FALLBACK_COOKIE_KEY;
            }
            key
        }
        Err(err) => {
            sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                "auth session: 读取凭据库失败({err}), 回退内置过渡密钥"
            )));
            fallback()
        }
    }
}

fn encode_key(key: &[u8; 32]) -> String {
    BASE64.encode(key)
}

fn decode_key(stored: &str) -> Option<[u8; 32]> {
    BASE64.decode(stored).ok()?.try_into().ok()
}

/// AES-256-GCM 加密, 每次写入独立随机 nonce
fn encrypt_cookies(key: &[u8; 32], plaintext: &str) -> Option<String> {
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = rand::random::<[u8; 12]>();
    let sealed = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .ok()?;
    Some(BASE64.encode([nonce.as_slice(), sealed.as_slice()].concat()))
}

fn decrypt_cookies(key: &[u8; 32], blob: &str) -> Option<String> {
    let raw = BASE64.decode(blob).ok()?;
    if raw.len() <= 12 {
        return None;
    }
    let (nonce, sealed) = raw.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let plaintext = cipher.decrypt(Nonce::from_slice(nonce), sealed).ok()?;
    String::from_utf8(plaintext).ok()
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

        let providers = match parsed.data {
            Some(blob) => {
                let Some(plain) = decrypt_cookies(&resolve_cookie_key(), &blob) else {
                    sidecar_log::spawn_runtime_log(serde_json::json!(
                        "auth session 解密失败(密钥不匹配或文件损坏), 按空处理"
                    ));
                    return HashMap::new();
                };
                serde_json::from_str::<HashMap<ProviderId, String>>(&plain).unwrap_or_default()
            }
            None => parsed.providers.unwrap_or_default(),
        };

        providers
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
        let encrypted = serde_json::to_string(&cookies)
            .ok()
            .and_then(|plain| encrypt_cookies(&resolve_cookie_key(), &plain));
        let Some(data) = encrypted else {
            sidecar_log::spawn_runtime_log(serde_json::json!("auth session 加密失败, 本次未写盘"));
            return;
        };
        let body = PersistedProviderSessions {
            version: Some(2),
            providers: None,
            data: Some(data),
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

    /// 测试固定密钥: 用例不触碰真实系统凭据库, 且进程内各用例保持一致
    fn seed_fixed_key() {
        let _ = COOKIE_KEY.set([0x5a; 32]);
    }

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
        seed_fixed_key();
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
        seed_fixed_key();
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

    /// Cookie 落盘必须是密文, 且解密后可完整往返
    #[tokio::test]
    async fn cookie_file_is_encrypted_at_rest() {
        let file = temp_cookie_file("encrypted");
        seed_fixed_key();
        let session = AuthSession::with_cookie_file(Some(file.clone()));

        session
            .set_runtime_provider_cookie(ProviderId::Netease, "MUSIC_U=secret123".to_owned())
            .await
            .unwrap();

        let raw = std::fs::read_to_string(&file).unwrap();
        assert!(!raw.contains("secret123"), "明文不得落盘");
        assert!(raw.contains("\"data\""));

        assert_eq!(
            session
                .get_provider_cookie(&ProviderId::Netease)
                .await
                .as_deref(),
            Some("MUSIC_U=secret123")
        );

        let _ = std::fs::remove_file(&file);
    }

    /// v1 明文旧文件可读; 下一次写入升级为 v2 密文格式
    #[tokio::test]
    async fn legacy_plaintext_file_is_read_then_upgraded() {
        let file = temp_cookie_file("legacy");
        seed_fixed_key();
        std::fs::write(&file, r#"{"version":1,"providers":{"netease":"a=1"}}"#).unwrap();
        let session = AuthSession::with_cookie_file(Some(file.clone()));

        assert_eq!(
            session
                .get_provider_cookie(&ProviderId::Netease)
                .await
                .as_deref(),
            Some("a=1")
        );

        session
            .set_runtime_provider_cookie(ProviderId::Soda, "b=2".to_owned())
            .await
            .unwrap();

        let raw = std::fs::read_to_string(&file).unwrap();
        assert!(!raw.contains("\"providers\":{"), "升级后不得再存明文字段");
        assert_eq!(
            session
                .get_provider_cookie(&ProviderId::Netease)
                .await
                .as_deref(),
            Some("a=1")
        );
        assert_eq!(
            session
                .get_provider_cookie(&ProviderId::Soda)
                .await
                .as_deref(),
            Some("b=2")
        );

        let _ = std::fs::remove_file(&file);
    }

    /// 密文损坏(密钥不匹配/文件破坏)按空处理, 不 panic 且可继续写入自愈
    #[tokio::test]
    async fn corrupted_blob_treated_as_empty() {
        let file = temp_cookie_file("corrupt");
        seed_fixed_key();
        std::fs::write(&file, r#"{"version":2,"data":"!!not-base64!!"}"#).unwrap();
        let session = AuthSession::with_cookie_file(Some(file.clone()));

        assert_eq!(session.get_provider_cookie(&ProviderId::Qq).await, None);

        session
            .set_runtime_provider_cookie(ProviderId::Qq, "c=3".to_owned())
            .await
            .unwrap();
        assert_eq!(
            session
                .get_provider_cookie(&ProviderId::Qq)
                .await
                .as_deref(),
            Some("c=3")
        );

        let _ = std::fs::remove_file(&file);
    }
}
