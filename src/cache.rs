use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::{sidecar_log, types::ProviderId};

pub(crate) const _TTL_10_MINUTES: Duration = Duration::from_secs(10 * 60);
pub(crate) const _TTL_1_HOUR: Duration = Duration::from_secs(60 * 60);
pub(crate) const TTL_1_DAY: Duration = Duration::from_secs(24 * 60 * 60);

const MIN_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_TTL: Duration = Duration::from_secs(36 * 60 * 60);
static CACHE: OnceLock<Cache> = OnceLock::new();

struct Cache {
    file_path: PathBuf,
    lock: Mutex<()>,
}

#[derive(Deserialize, Serialize)]
struct CacheValue {
    dead_at: u64,
    raw: String,
}

type CacheContents = BTreeMap<String, BTreeMap<String, CacheValue>>;

pub(crate) fn configure(data_dir: PathBuf) -> Result<(), &'static str> {
    let cache = Cache {
        file_path: data_dir.join("cache.json"),
        lock: Mutex::new(()),
    };

    if let Some(existing) = CACHE.get() {
        return if existing.file_path == cache.file_path {
            Ok(())
        } else {
            Err("cache has already been initialized with a different path")
        };
    }

    CACHE
        .set(cache)
        .map_err(|_| "cache has already been initialized")
}

pub(crate) async fn get(provider: ProviderId, key: &str) -> Option<String> {
    let cache = CACHE.get()?;
    let _guard = cache.lock.lock().await;
    read_cache(&cache.file_path)
        .get(provider.as_str())
        .and_then(|entries| entries.get(key))
        .filter(|value| value.dead_at > now())
        .map(|value| value.raw.clone())
}

pub(crate) async fn insert(provider: ProviderId, key: &str, ttl: Duration, raw: String) {
    let Some(cache) = CACHE.get() else {
        return;
    };
    let Some(dead_at) = dead_at(ttl) else {
        return;
    };
    let _guard = cache.lock.lock().await;
    let mut contents = read_cache(&cache.file_path);
    contents
        .entry(provider.as_str().to_owned())
        .or_default()
        .insert(key.to_owned(), CacheValue { dead_at, raw });
    write_cache(&cache.file_path, &contents);
}

pub(crate) async fn get_or_refresh<E, F, Fut>(
    provider: ProviderId,
    key: &str,
    ttl: Duration,
    refresh: F,
) -> Result<Option<String>, E>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<String>, E>>,
{
    if let Some(raw) = get(provider, key).await {
        return Ok(Some(raw));
    }
    let raw = refresh().await?;
    if let Some(raw) = raw.as_ref() {
        insert(provider, key, ttl, raw.clone()).await;
    }
    Ok(raw)
}

pub(crate) async fn remove(provider: ProviderId, key: &str) {
    let Some(cache) = CACHE.get() else {
        return;
    };
    let _guard = cache.lock.lock().await;
    let mut contents = read_cache(&cache.file_path);
    if contents
        .get_mut(provider.as_str())
        .is_some_and(|entries| entries.remove(key).is_some())
    {
        write_cache(&cache.file_path, &contents);
    }
}

fn dead_at(ttl: Duration) -> Option<u64> {
    (ttl > MIN_TTL && ttl <= MAX_TTL).then(|| now().saturating_add(ttl.as_secs()))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_cache(file_path: &PathBuf) -> CacheContents {
    fs::read_to_string(file_path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

fn write_cache(file_path: &PathBuf, contents: &CacheContents) {
    if let Some(parent) = file_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(contents) {
        if let Err(err) = fs::write(file_path, json) {
            sidecar_log::spawn_runtime_log(serde_json::json!(format!(
                "Cache 写入失败: {err}"
            )));
        }
    }
}
