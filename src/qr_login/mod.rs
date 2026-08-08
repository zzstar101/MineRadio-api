use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;

use crate::types::{ProviderId, ProviderLoginQrCheck, ProviderLoginQrImage, ProviderLoginQrKey};

pub(crate) mod common;
pub(crate) mod kugou;
pub(crate) mod mqtt;
pub(crate) mod netease;
pub(crate) mod qq;
pub(crate) mod qq_music;
pub(crate) mod soda;
pub(crate) mod wechat;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum QrLoginKind {
    Qq,
    QqMusic,
    Wechat,
    Netease,
    Kugou,
    Soda,
}

impl QrLoginKind {
    pub const ALL: [Self; 6] = [
        Self::Qq,
        Self::QqMusic,
        Self::Wechat,
        Self::Netease,
        Self::Kugou,
        Self::Soda,
    ];

    pub const fn provider(self) -> ProviderId {
        match self {
            Self::Qq | Self::QqMusic | Self::Wechat => ProviderId::Qq,
            Self::Netease => ProviderId::Netease,
            Self::Kugou => ProviderId::Kugou,
            Self::Soda => ProviderId::Soda,
        }
    }
}

pub const QR_LOGIN_SESSION_TTL: Duration = Duration::from_secs(30);

/// 所有二维码登录服务对路由层提供的统一操作契约。
///
/// 具体协议只负责生成二维码、轮询上游状态和保存自己的协议状态；路由层
/// 不需要知道 QQ、网易云、酷狗或 Soda 的请求细节。
#[async_trait]
pub trait QrLogin: Send + Sync {
    async fn create_key(&self) -> Result<ProviderLoginQrKey>;
    async fn create_image(&self, key: &str) -> Result<ProviderLoginQrImage>;
    async fn check(&self, key: &str) -> Result<ProviderLoginQrCheck>;
}

/// 一张二维码登录会话。
///
/// `image` 是前端展示所需的二维码图片，`state` 是协议专属状态：例如 QQ
/// 的 cookie、微信的 uuid、QQ 音乐 MQTT 的连接对象，以及酷狗的设备参数。
/// 通过泛型保留这些状态，公共层不需要理解任何具体协议。
pub struct QrSession<S> {
    pub image: String,
    pub state: S,
}

/// Store 内部的条目，同时记录创建时间用于 TTL 清理。
struct StoredQrSession<S> {
    session: Arc<Mutex<QrSession<S>>>,
    created_at: Instant,
}

/// 通用二维码会话存储。
///
/// 每个扫码服务持有自己的 `QrSessionStore<S>`，所以不同 Provider 或不同
/// 登录协议之间不会共享 key 和状态。`S` 由适配器决定，既能承载复杂的
/// 协议状态，也能像 Soda 一样只存图片而不额外保存状态。
///
/// Store 自带 30 秒 TTL 清理：第一次插入会话时启动一个清理任务，任务每
/// 30 秒清理过期条目；当 HashMap 为空时任务自动退出，下一次插入时再启动。
/// 这样不需要为每个二维码服务单独维护后台清理线程。
pub struct QrSessionStore<S> {
    sessions: Arc<Mutex<HashMap<String, StoredQrSession<S>>>>,
    cleanup_running: Arc<Mutex<bool>>,
}

impl<S> Default for QrSessionStore<S> {
    fn default() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            cleanup_running: Arc::new(Mutex::new(false)),
        }
    }
}

impl<S: Send + 'static> QrSessionStore<S> {
    pub async fn insert(&self, key: String, image: String, state: S) {
        self.sessions.lock().await.insert(
            key,
            StoredQrSession {
                session: Arc::new(Mutex::new(QrSession { image, state })),
                created_at: Instant::now(),
            },
        );
        self.ensure_cleanup_task().await;
    }

    pub async fn get(&self, key: &str) -> Option<Arc<Mutex<QrSession<S>>>> {
        self.sessions
            .lock()
            .await
            .get(key)
            .map(|entry| Arc::clone(&entry.session))
    }

    pub async fn remove(&self, key: &str) {
        self.sessions.lock().await.remove(key);
    }

    async fn ensure_cleanup_task(&self) {
        let mut running = self.cleanup_running.lock().await;
        if *running {
            return;
        }
        *running = true;

        let sessions = Arc::clone(&self.sessions);
        let cleanup_running = Arc::clone(&self.cleanup_running);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(QR_LOGIN_SESSION_TTL).await;

                let empty = {
                    let mut sessions = sessions.lock().await;
                    let now = Instant::now();
                    let expired_keys = sessions
                        .iter()
                        .filter(|(_, session)| {
                            now.duration_since(session.created_at) >= QR_LOGIN_SESSION_TTL
                        })
                        .map(|(key, _)| key.clone())
                        .collect::<Vec<_>>();
                    for key in &expired_keys {
                        sessions.remove(key);
                    }
                    sessions.is_empty()
                };

                if empty {
                    let mut running = cleanup_running.lock().await;
                    if sessions.lock().await.is_empty() {
                        *running = false;
                        break;
                    }
                }
            }
        });
    }
}
