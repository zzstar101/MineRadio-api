//! 锁中毒的统一恢复策略: 不 panic、不向上传播, 记一条运行时日志后带伤继续。
//!
//! std 的 Mutex/RwLock 在持锁线程 panic 后会进入中毒状态; 本crate选择
//! 宁可带着可能不一致的状态运行, 也不要让单个故障放大成整进程崩溃。

use std::sync::PoisonError;

use crate::sidecar_log;

/// 用作 `unwrap_or_else` 的恢复闭包
pub(crate) fn continue_on_poison<T>(poisoned: PoisonError<T>) -> T {
    sidecar_log::spawn_runtime_log(serde_json::json!(format!(
        "锁中毒, 恢复后继续执行: {poisoned}"
    )));
    poisoned.into_inner()
}
