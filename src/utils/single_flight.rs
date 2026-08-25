//! 读穿缓存 + 同 key 单飞闸门(single-flight)
//!
//! 流程: 查缓存(新鲜且覆盖区间) → 未命中则登记容量需求并排队 → 持锁者双检并
//! 合并排队者累积的最大需求 → 按「目标容量 + 预取余量」拉取 → 写回并切片返回。
//!
//! 失败不向等待者扩散: 持锁者重试耗尽后带着错误释放闸门,
//! 下一个排队者成为新持锁者时会重新发起请求。
//! 瞬时故障下第二次尝试往往就成功, 扩散反而会把局部抖动放大成整批失败;
//! 若上游确定性坏掉, 排队者各自撞一次墙的代价也可接受且有重试上限兜底。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::providers::{
    ProviderResult,
    error::{ProviderError, ProviderErrorCode},
};
use crate::types::ProviderId;

/// 缓存条目存活时长
const CACHE_TTL: Duration = Duration::from_secs(300);
/// 每次拉取在目标容量之外追加的余量倍数(× batch), 为后续顺序翻页预取
const HEADROOM_BATCHES: u32 = 5;

/// 参与分页读穿缓存的集合: 提供总条数与机械切片(越界给空)
pub trait Paginated: Clone {
    /// 覆盖判断所依据的总条目数
    fn total(&self) -> usize;

    /// 机械切片 [start, end); 不负责 has_more 等语义字段
    fn slice_range(&self, start: usize, end: usize) -> Self;
}

struct Slot<V> {
    entry: Option<(V, Instant)>,
    gate: Arc<tokio::sync::Mutex<()>>,
    /// 排队者注册的最大容量需求(offset+limit), 由下一个持锁者一次性消费
    demand: u32,
}

impl<V> Default for Slot<V> {
    fn default() -> Self {
        Self {
            entry: None,
            gate: Arc::default(),
            demand: 0,
        }
    }
}

/// 已按请求区间切好的一页缓存数据
pub struct CachedPage<V> {
    pub value: V,
    pub has_more: bool,
}

/// 默认扩容步长与重试次数
pub const DEFAULT_BATCH: u32 = 200;
pub const DEFAULT_RETRIES: u32 = 2;

/// 同 key 并发请求只放一个去拉取, 其余排队共享结果的读穿缓存。
///
/// - 快路径不碰闸门: 缓存新鲜且覆盖 `[offset, offset+limit)` 时直接切片返回
/// - 慢路径先把自身需求合并进槽位再排队; 持锁者取走累积的最大需求作为拉取目标,
///   追加 `HEADROOM_BATCHES * batch` 余量后拉取, 保证一次请求尽量喂饱整个队列
/// - `fetch(key, capacity)` 返回的数据不足 capacity 时按实际条数截断
pub struct SingleFlightCache<V> {
    slots: Mutex<HashMap<String, Slot<V>>>,
    /// 扩容步长
    batch: u32,
    /// 拉取失败后的额外重试次数(总尝试 = 1 + retries)
    retries: u32,
    provider: ProviderId,
}

impl<V: Paginated> Default for SingleFlightCache<V> {
    fn default() -> Self {
        Self::new(ProviderId::default(), DEFAULT_BATCH, DEFAULT_RETRIES)
    }
}

impl<V: Paginated> SingleFlightCache<V> {
    pub fn new(provider: ProviderId, batch: u32, retries: u32) -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            batch,
            retries,
            provider,
        }
    }

    pub async fn get<F, Fut>(
        &self,
        key: &str,
        offset: u32,
        limit: u32,
        fetch: F,
    ) -> ProviderResult<CachedPage<V>>
    where
        F: Fn(String, u32) -> Fut,
        Fut: std::future::Future<Output = ProviderResult<V>>,
    {
        let need = offset.saturating_add(limit);

        // 快路径: 新鲜且覆盖请求区间时直接切片, 不进闸门排队
        if let Some(page) = self.cached_page(key, need, offset, limit) {
            return Ok(page);
        }

        // 登记容量需求后取该 key 的单飞闸门(每 key 一把, 首次访问时创建)
        let gate = {
            let mut slots = self
                .slots
                .lock()
                .unwrap_or_else(crate::utils::poison::continue_on_poison);
            let slot = slots.entry(key.to_owned()).or_default();
            slot.demand = slot.demand.max(need);
            Arc::clone(&slot.gate)
        };
        let _permit = gate.lock().await;

        // 合并排队期间累积的最大需求, 一次请求尽量喂饱整个队列
        let target = {
            let mut slots = self
                .slots
                .lock()
                .unwrap_or_else(crate::utils::poison::continue_on_poison);
            let slot = slots.entry(key.to_owned()).or_default();
            let merged = slot.demand.max(need);
            slot.demand = 0;
            merged
        };

        // 双检: 排队期间前一个请求可能已经把数据取回
        if let Some(page) = self.cached_page(key, target, offset, limit) {
            return Ok(page);
        }

        // 目标容量 + 预取余量; 失败在重试次数内自旋, 重试耗尽也不扩散给等待者
        let want = target.saturating_add(self.batch.saturating_mul(HEADROOM_BATCHES));
        let mut last_err = None;
        for _ in 0..=self.retries {
            match fetch(key.to_owned(), want).await {
                Ok(value) => {
                    let total = value.total();
                    {
                        let mut slots = self
                            .slots
                            .lock()
                            .unwrap_or_else(crate::utils::poison::continue_on_poison);
                        slots.entry(key.to_owned()).or_default().entry =
                            Some((value.clone(), Instant::now() + CACHE_TTL));
                    }
                    return Ok(slice_page(value, offset, limit, total));
                }
                Err(err) => last_err = Some(err),
            }
        }

        Err(last_err.unwrap_or_else(|| ProviderError {
            code: ProviderErrorCode::Internal,
            provider: self.provider,
            message: "single flight cache exhausted without error".to_owned(),
            retryable: false,
            action: None,
            raw_message: None,
        }))
    }

    /// 缓存新鲜且总条数足以覆盖 need 时, 返回 [offset, offset+limit) 切片页
    fn cached_page(&self, key: &str, need: u32, offset: u32, limit: u32) -> Option<CachedPage<V>> {
        let slots = self
            .slots
            .lock()
            .unwrap_or_else(crate::utils::poison::continue_on_poison);
        let (value, expires_at) = slots.get(key)?.entry.as_ref()?;
        if *expires_at <= Instant::now() || value.total() < need as usize {
            return None;
        }
        Some(slice_page(value.clone(), offset, limit, value.total()))
    }
}

fn slice_page<V: Paginated>(value: V, offset: u32, limit: u32, total: usize) -> CachedPage<V> {
    let start = offset as usize;
    let end = start.saturating_add(limit as usize).min(total);
    CachedPage {
        value: value.slice_range(start, end),
        has_more: offset.saturating_add(limit) < total as u32,
    }
}
