//! 全局缓存守卫: 周期清扫登记过的内存缓存与磁盘 cache.json。
//!
//! - 登记表只存 `Weak`, adapter 被丢弃后 upgrade 失败即剔除, 守卫不延长任何组件寿命
//! - 守卫任务在首次异步缓存调用时拉起(构造可能发生在同步上下文)
//! - 巡检周期与 single_flight 的条目 TTL 对齐, 死条目最长滞留 ≈ TTL + PERIOD

use std::{
    sync::{LazyLock, Mutex, OnceLock, Weak},
    time::Duration,
};

use crate::{cache, sidecar_log};

/// 巡检周期
const PERIOD: Duration = Duration::from_secs(300);

/// 可被守卫周期清扫的缓存
pub(crate) trait Sweepable: Send + Sync {
    /// 执行一次清扫, 返回清理条数(用于汇总日志)
    fn sweep(&self) -> usize;
}

static REGISTRY: LazyLock<Mutex<Vec<Weak<dyn Sweepable>>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

static STARTED: OnceLock<()> = OnceLock::new();

/// 缓存构造完成后登记(Arc 降级为 Weak), 只登记不拉起任务
pub(crate) fn register(cache: Weak<dyn Sweepable>) {
    REGISTRY
        .lock()
        .unwrap_or_else(crate::utils::poison::continue_on_poison)
        .push(cache);
}

/// 首次异步调用时调用一次; 必须处于 tokio runtime 内
pub(crate) fn ensure_spawned() {
    if STARTED.set(()).is_ok() {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(PERIOD);
            ticker.tick().await; // interval 首个 tick 立即完成, 跳过
            loop {
                ticker.tick().await;
                run_once().await;
            }
        });
    }
}

async fn run_once() {
    let mut swept = 0;

    // 不持锁跨 await: 先同步扫完内存缓存再释放注册表锁
    {
        let mut registry = REGISTRY
            .lock()
            .unwrap_or_else(crate::utils::poison::continue_on_poison);
        registry.retain(|weak| match weak.upgrade() {
            Some(cache) => {
                swept += cache.sweep();
                true
            }
            None => false,
        });
    }

    swept += cache::sweep_expired().await;

    if swept > 0 {
        sidecar_log::spawn_runtime_log(serde_json::json!(format!("缓存守卫清扫 {swept} 条")));
    }
}
