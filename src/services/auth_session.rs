use std::{collections::HashMap, path::PathBuf};

use tokio::sync::RwLock;

use crate::types::ProviderId;

#[derive(Debug)]
pub struct AuthSession {
    runtime: RwLock<HashMap<ProviderId, String>>,
    persisted: Option<PathBuf>,
}

impl AuthSession {
    pub fn new(persisted: Option<PathBuf>) -> Self {
        Self {
            runtime: RwLock::new(HashMap::new()),
            persisted,
        }
    }

    pub async fn get_cookie(&self, provider: &str) -> Option<String> {
        self.runtime.read().await.get(provider).cloned()
    }

    pub async fn set_cookie(&self, provider: ProviderId, cookie: String) {
        self.runtime.write().await.insert(provider, cookie);
    }

    pub fn persisted_path(&self) -> Option<&PathBuf> {
        self.persisted.as_ref()
    }
}
