use std::{
    collections::{BTreeMap, HashMap},
    fs,
    future::Future,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex as StdMutex, OnceLock},
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

/// 刷新去重闸门: 同 (provider, key) 的并发刷新只放一个真正执行, 其余排队后
/// 通过双检直接吃缓存结果。键数量有限(目前仅各 provider 的推荐页), 不做回收。
static REFRESH_GATES: LazyLock<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

fn refresh_gate(provider: ProviderId, key: &str) -> Arc<tokio::sync::Mutex<()>> {
    let gate_key = format!("{}:{key}", provider.as_str());
    let mut gates = REFRESH_GATES.lock().unwrap_or_else(|e| e.into_inner());
    Arc::clone(gates.entry(gate_key).or_default())
}

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

    // 同 key 并发 miss 只放一个去刷新, 其余在闸门上排队
    let gate = refresh_gate(provider, key);
    let _guard = gate.lock().await;

    // 双检: 排队期间前一个请求可能已经刷新完成
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
    if ttl > MIN_TTL && ttl <= MAX_TTL {
        return Some(now().saturating_add(ttl.as_secs()));
    }
    // 区间外的 TTL 会让本次写入被静默跳过, 必须留痕否则调用方无从察觉
    sidecar_log::spawn_runtime_log(serde_json::json!(format!(
        "cache: TTL {ttl:?} 超出允许区间 [{MIN_TTL:?}, {MAX_TTL:?}], 本次写入忽略"
    )));
    None
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
        // 先写临时文件再原子替换, 写一半崩溃也不会留下截断的 JSON
        let tmp = file_path.with_extension("tmp");
        if let Err(err) = fs::write(&tmp, json).and_then(|()| fs::rename(&tmp, file_path)) {
            sidecar_log::spawn_runtime_log(serde_json::json!(format!("Cache 写入失败: {err}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// CACHE 是进程级 OnceLock, 相关用例必须共用同一路径并合并在单个测试里
    #[tokio::test]
    async fn refresh_dedup_and_ttl_bounds() {
        let dir = std::env::temp_dir().join(format!("mineradio_cache_test_{}", std::process::id()));
        configure(dir.clone()).expect("configure cache");

        // 同 key 并发刷新只允许一次真实上游调用: 刷新期间让出执行权,
        // 强制第二个请求在闸门上排队后走双检命中
        let calls = Arc::new(AtomicU32::new(0));
        let make_refresh = |calls: Arc<AtomicU32>| {
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Ok::<_, ()>(Some("value".to_owned()))
                }
            }
        };

        let (a, b) = tokio::join!(
            get_or_refresh(
                ProviderId::Netease,
                "dedup_test",
                TTL_1_DAY,
                make_refresh(Arc::clone(&calls))
            ),
            get_or_refresh(
                ProviderId::Netease,
                "dedup_test",
                TTL_1_DAY,
                make_refresh(Arc::clone(&calls))
            ),
        );
        a.unwrap();
        b.unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        // 区间外的 TTL 拒绝写入且不落盘
        insert(
            ProviderId::Soda,
            "short_ttl",
            Duration::from_secs(1),
            "x".to_owned(),
        )
        .await;
        assert_eq!(get(ProviderId::Soda, "short_ttl").await, None);
        insert(
            ProviderId::Soda,
            "long_ttl",
            Duration::from_secs(48 * 60 * 60),
            "y".to_owned(),
        )
        .await;
        assert_eq!(get(ProviderId::Soda, "long_ttl").await, None);

        // 区间内的正常写入可读回
        insert(ProviderId::Soda, "valid_ttl", TTL_1_DAY, "z".to_owned()).await;
        assert_eq!(
            get(ProviderId::Soda, "valid_ttl").await.as_deref(),
            Some("z")
        );

        let _ = std::fs::remove_file(dir.join("cache.json"));
        let _ = std::fs::remove_dir(dir);
    }
}
