//! 压栈弹出式预取队列: 电台/私人 FM 类「流式下一曲」的读穿缓存。
//!
//! 单飞骨架与 single_flight 同构: 快路径(std 锁弹出, 不碰闸门) → 低水位或弹空时
//! 登记需求并排队 → 持闸者取走累积需求, 按「基础批量 + 排队人数」拉取 → 双检后
//! 补货弹出。失败不向等待者扩散: 手里有候补的照样返回, 两手空空的带着错误出闸,
//! 下一位排队者成为新持闸者重新拉取 —— 瞬时故障下第二次尝试往往就成功。

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{
    providers::{
        ProviderResult,
        error::{ProviderError, ProviderErrorCode},
    },
    types::ProviderId,
    utils::poison::continue_on_poison,
};

/// 每次拉取的基础条数; 闸门上每多一个排队者追加一条
pub const BASE_BATCH: u32 = 5;
/// 弹出后剩余条数 ≤ 低水位即触发补货; 默认留 2 条兜底,
/// 补货持续失败导致队列清空前, 冷启动路径始终有种子 id 可用
pub const DEFAULT_LOW_WATER: usize = 2;
/// 拉取失败后的额外重试次数(总尝试 = 1 + retries)
pub const DEFAULT_RETRIES: u32 = 2;

struct Slot<V> {
    queue: Vec<V>,
    gate: Arc<tokio::sync::Mutex<()>>,
    /// 排队者登记的追加条数, 由持闸者一次性取走
    demand: u32,
}

impl<V> Default for Slot<V> {
    fn default() -> Self {
        Self {
            queue: Vec::new(),
            gate: Arc::default(),
            demand: 0,
        }
    }
}

pub struct PopQueue<V> {
    slots: Mutex<HashMap<String, Slot<V>>>,
    provider: ProviderId,
    low_water: usize,
    retries: u32,
}

impl<V> Default for PopQueue<V> {
    fn default() -> Self {
        Self::new(ProviderId::default(), DEFAULT_LOW_WATER, DEFAULT_RETRIES)
    }
}

impl<V> PopQueue<V> {
    pub fn new(provider: ProviderId, low_water: usize, retries: u32) -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            provider,
            low_water,
            retries,
        }
    }

    /// 弹出该 key 的下一个条目。
    ///
    /// - `fetch(seed, want)` 仅在缺货/低水位时被调用: 冷启动 `seed` 为 `None`
    ///   (种子由闭包自行捕获), 低水位补货为 `Some(&刚弹出的条目)`; `want` 为
    ///   建议拉取条数(基础批量 + 排队人数), 上游给多少存多少
    /// - 快路径有余量时完全不碰闸门与上游
    pub async fn pop<F, Fut>(&self, key: &str, fetch: F) -> ProviderResult<V>
    where
        F: Fn(Option<&V>, u32) -> Fut,
        Fut: Future<Output = ProviderResult<Vec<V>>>,
    {
        // 快路径: 有余量直接走; 低水位的带着刚弹出的条目去排队补货
        let mut candidate = None;
        {
            let mut slots = self.slots.lock().unwrap_or_else(continue_on_poison);
            if let Some(slot) = slots.get_mut(key)
                && let Some(item) = slot.queue.pop()
            {
                if slot.queue.len() <= self.low_water {
                    candidate = Some(item);
                } else {
                    return Ok(item);
                }
            }
        }

        // 登记 +1 需求后进单飞闸门(每 key 一把, 首次访问时创建)
        let gate = {
            let mut slots = self.slots.lock().unwrap_or_else(continue_on_poison);
            let slot = slots.entry(key.to_owned()).or_default();
            slot.demand += 1;
            Arc::clone(&slot.gate)
        };
        let _permit = gate.lock().await;

        // 取走累积需求(含自己, 减一即纯排队者), 并双检: 排队期间前一位持闸者可能已补满
        let (want, stale) = {
            let mut slots = self.slots.lock().unwrap_or_else(continue_on_poison);
            let slot = slots.entry(key.to_owned()).or_default();
            let extra = std::mem::take(&mut slot.demand).saturating_sub(1);
            let remaining = slot.queue.len();
            let stale = match &candidate {
                Some(_) => remaining > self.low_water,
                None => remaining != 0,
            };
            (BASE_BATCH.saturating_add(extra), stale)
        };

        let mut last_err = None;
        if !stale {
            for _ in 0..=self.retries {
                match fetch(candidate.as_ref(), want).await {
                    Ok(batch) => {
                        let mut slots = self.slots.lock().unwrap_or_else(continue_on_poison);
                        slots.entry(key.to_owned()).or_default().queue.extend(batch);
                        break;
                    }
                    Err(err) => last_err = Some(err),
                }
            }
        }

        match candidate {
            // 手里有候补: 补货失败也不影响本次返回
            Some(item) => Ok(item),
            None => {
                let mut slots = self.slots.lock().unwrap_or_else(continue_on_poison);
                slots
                    .get_mut(key)
                    .and_then(|slot| slot.queue.pop())
                    .ok_or_else(|| no_items_error(self.provider, key, last_err))
            }
        }
    }

    /// 预览该 key 下一个将弹出的条目但不移除; 空队列返回 `None`。
    ///
    /// 只读展示用途(如追加标题), 同步且不触发上游拉取;
    /// 消费请走 [`Self::pop`] —— 它等价于 get 之后删除
    pub fn get(&self, key: &str) -> Option<V>
    where
        V: Clone,
    {
        let slots = self.slots.lock().unwrap_or_else(continue_on_poison);
        slots.get(key)?.queue.last().cloned()
    }

    /// 预览该 key 的下一个条目但不消费; 队列有货直接返回,
    /// 弹空时经与 [`Self::pop`] 相同的单飞闸门补货后再预览。
    ///
    /// 展示预热场景用(如推荐页为电台卡补封面与标题): 客户端随后真实播放
    /// 走 [`Self::pop`] 时, 消费的正是这里预热的同一批
    pub async fn peek<F, Fut>(&self, key: &str, fetch: F) -> ProviderResult<V>
    where
        V: Clone,
        F: Fn(Option<&V>, u32) -> Fut,
        Fut: Future<Output = ProviderResult<Vec<V>>>,
    {
        if let Some(item) = self.get(key) {
            return Ok(item);
        }

        // 与 pop 同构: 登记需求 → 过闸 → 取走累积需求定批量 → 双检 → 补货
        let gate = {
            let mut slots = self.slots.lock().unwrap_or_else(continue_on_poison);
            let slot = slots.entry(key.to_owned()).or_default();
            slot.demand += 1;
            Arc::clone(&slot.gate)
        };
        let _permit = gate.lock().await;

        let (want, stale) = {
            let mut slots = self.slots.lock().unwrap_or_else(continue_on_poison);
            let slot = slots.entry(key.to_owned()).or_default();
            let extra = std::mem::take(&mut slot.demand).saturating_sub(1);
            (BASE_BATCH.saturating_add(extra), !slot.queue.is_empty())
        };

        let mut last_err = None;
        if !stale {
            for _ in 0..=self.retries {
                match fetch(None, want).await {
                    Ok(batch) => {
                        let mut slots = self.slots.lock().unwrap_or_else(continue_on_poison);
                        slots.entry(key.to_owned()).or_default().queue.extend(batch);
                        break;
                    }
                    Err(err) => last_err = Some(err),
                }
            }
        }

        self.get(key)
            .ok_or_else(|| no_items_error(self.provider, key, last_err))
    }

    /// 清空全部 key(登出/守卫清扫用), 返回清理条数
    pub fn clear(&self) -> usize {
        let mut slots = self.slots.lock().unwrap_or_else(continue_on_poison);
        let count = slots.values().map(|slot| slot.queue.len()).sum();
        slots.clear();
        count
    }
}

/// 补货后仍无货的统一错误: 有上游错误则透传, 否则按 NoResult 兜底
fn no_items_error(
    provider: ProviderId,
    key: &str,
    last_err: Option<ProviderError>,
) -> ProviderError {
    last_err.unwrap_or_else(|| ProviderError {
        code: ProviderErrorCode::NoResult,
        provider,
        message: format!("pop queue fetch returned no items for {key}"),
        retryable: false,
        action: None,
        raw_message: None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;

    type Recorder = Arc<StdMutex<Vec<(u32, Option<i32>)>>>;

    /// 记录 (want, seed) 后返回 [start, start+want) 的假批次
    fn recording_fetch(
        recorder: &Recorder,
        start: i32,
    ) -> impl Fn(Option<&i32>, u32) -> std::future::Ready<ProviderResult<Vec<i32>>> + Send + '_
    {
        move |seed, want| {
            recorder.lock().unwrap().push((want, seed.copied()));
            let batch = (start..start + want as i32).collect::<Vec<_>>();
            std::future::ready(Ok(batch))
        }
    }

    #[tokio::test]
    async fn cold_pop_fetches_base_batch_and_drains_without_upstream() {
        let queue = PopQueue::<i32>::default();
        let recorder = Recorder::default();

        // 冷启动: 拉 BASE_BATCH 条, 弹出末尾一个
        assert_eq!(
            queue
                .pop("k", recording_fetch(&recorder, 10))
                .await
                .unwrap(),
            14
        );
        assert_eq!(*recorder.lock().unwrap(), vec![(5, None)]);

        // 余量 3 > 低水位 2: 纯快路径, 不碰上游
        assert_eq!(
            queue
                .pop("k", |_, _| std::future::ready(Err(bad())))
                .await
                .unwrap(),
            13
        );

        // 余量降到低水位: 带着候选补货一次, 种子是刚弹出的那条
        assert_eq!(
            queue
                .pop("k", recording_fetch(&recorder, 20))
                .await
                .unwrap(),
            12
        );
        assert_eq!(*recorder.lock().unwrap(), vec![(5, None), (5, Some(12))]);
        // 新批次接在旧队列尾部, 继续消费
        assert_eq!(
            queue
                .pop("k", |_, _| std::future::ready(Err(bad())))
                .await
                .unwrap(),
            24
        );
    }

    #[tokio::test]
    async fn queued_waiters_expand_one_shared_fetch() {
        let queue = Arc::new(PopQueue::<i32>::default());
        // 预先占住闸门, 让两个 pop 都排队, 需求合并到先入者
        let gate = {
            let mut slots = queue.slots.lock().unwrap();
            Arc::clone(&slots.entry("k".to_owned()).or_default().gate)
        };
        let permit = gate.lock().await;

        let wants = Recorder::default();
        let h1 = {
            let queue = Arc::clone(&queue);
            let wants = Arc::clone(&wants);
            tokio::spawn(async move {
                queue
                    .pop("k", move |_, want| {
                        wants.lock().unwrap().push((want, None));
                        std::future::ready(Ok((10..10 + want as i32).collect::<Vec<_>>()))
                    })
                    .await
            })
        };
        let h2 = {
            let queue = Arc::clone(&queue);
            tokio::spawn(async move {
                // 若该闭包被调用说明上游被重复拉取, 直接让测试失败
                queue.pop("k", |_, _| std::future::ready(Err(bad()))).await
            })
        };

        // 等两位都完成需求登记
        for _ in 0..1000 {
            let ready = queue
                .slots
                .lock()
                .unwrap()
                .get("k")
                .is_some_and(|s| s.demand == 2);
            if ready {
                break;
            }
            tokio::task::yield_now().await;
        }
        drop(permit);

        // 持闸者按「基础 5 + 排队 1」拉取, 两位都从同一批里弹出
        assert_eq!(h1.await.unwrap().unwrap(), 15);
        assert_eq!(h2.await.unwrap().unwrap(), 14);
        assert_eq!(*wants.lock().unwrap(), vec![(6, None)]);
        assert_eq!(queue.clear(), 4);
    }

    #[tokio::test]
    async fn candidate_survives_fetch_failure() {
        let queue = PopQueue::<i32>::new(ProviderId::Qq, DEFAULT_LOW_WATER, 0);
        queue.extend_for_test("k", [1, 2, 3]);
        // 弹出后余量到达低水位, 补货失败也不影响本次返回
        assert_eq!(
            queue
                .pop("k", |_, _| std::future::ready(Err(bad())))
                .await
                .unwrap(),
            3
        );
        assert_eq!(queue.clear(), 2);
    }

    #[tokio::test]
    async fn cold_failure_retries_then_propagates() {
        let queue = PopQueue::<i32>::new(ProviderId::Qq, DEFAULT_LOW_WATER, 2);
        let attempts = Arc::new(StdMutex::new(0));
        let counter = Arc::clone(&attempts);
        let err = queue
            .pop("k", move |_, _| {
                *counter.lock().unwrap() += 1;
                std::future::ready(Err(bad()))
            })
            .await
            .unwrap_err();
        assert!(matches!(err.code, ProviderErrorCode::Unavailable));
        assert_eq!(*attempts.lock().unwrap(), 3); // 1 + retries
    }

    #[tokio::test]
    async fn cold_empty_batch_reports_no_result() {
        let queue = PopQueue::<i32>::new(ProviderId::Qq, DEFAULT_LOW_WATER, 0);
        let err = queue
            .pop("k", |_, _| std::future::ready(Ok(Vec::new())))
            .await
            .unwrap_err();
        assert!(matches!(err.code, ProviderErrorCode::NoResult));
    }

    #[tokio::test]
    async fn get_peeks_without_removing() {
        let queue = PopQueue::<i32>::default();
        assert_eq!(queue.get("k"), None);

        queue.extend_for_test("k", [1, 2, 3, 4]);
        // 预览可重复, 始终指向下一个将弹出的条目
        assert_eq!(queue.get("k"), Some(4));
        assert_eq!(queue.get("k"), Some(4));

        // 弹出的正是预览到的那条, 且余量充足不触发上游
        let recorder = Recorder::default();
        assert_eq!(
            queue
                .pop("k", recording_fetch(&recorder, 99))
                .await
                .unwrap(),
            4
        );
        assert!(recorder.lock().unwrap().is_empty());
        assert_eq!(queue.get("k"), Some(3));
    }

    #[tokio::test]
    async fn peek_serves_from_stock_without_consuming() {
        let queue = PopQueue::<i32>::default();
        queue.extend_for_test("k", [1, 2]);
        // 有货直接预览, 不碰上游也不消费
        assert_eq!(
            queue
                .peek("k", |_, _| std::future::ready(Err(bad())))
                .await
                .unwrap(),
            2
        );
        assert_eq!(queue.get("k"), Some(2));
    }

    #[tokio::test]
    async fn peek_on_empty_fetches_and_preserves_for_pop() {
        let queue = PopQueue::<i32>::default();
        let recorder = Recorder::default();

        // 空队列经闸门补货后预览到首曲(末尾元素)
        assert_eq!(
            queue
                .peek("k", recording_fetch(&recorder, 10))
                .await
                .unwrap(),
            14
        );
        assert_eq!(*recorder.lock().unwrap(), vec![(5, None)]);

        // 预热的同一批留给 pop 消费, 且余量充足不再触发上游
        assert_eq!(
            queue
                .pop("k", |_, _| std::future::ready(Err(bad())))
                .await
                .unwrap(),
            14
        );
    }

    #[tokio::test]
    async fn peek_failure_propagates_when_cold() {
        let queue = PopQueue::<i32>::new(ProviderId::Qq, DEFAULT_LOW_WATER, 0);
        let err = queue
            .peek("k", |_, _| std::future::ready(Err(bad())))
            .await
            .unwrap_err();
        assert!(matches!(err.code, ProviderErrorCode::Unavailable));
    }

    #[test]
    fn clear_drops_all_keys_and_reports_count() {
        let queue = PopQueue::<i32>::default();
        queue.extend_for_test("a", [1, 2]);
        queue.extend_for_test("b", [3]);
        assert_eq!(queue.clear(), 3);
        assert_eq!(queue.clear(), 0);
    }

    /// 测试专用: 直塞一批入库(绕过 fetch)
    trait ExtendForTest {
        fn extend_for_test(&self, key: &str, items: impl IntoIterator<Item = i32>);
    }

    impl ExtendForTest for PopQueue<i32> {
        fn extend_for_test(&self, key: &str, items: impl IntoIterator<Item = i32>) {
            let mut slots = self.slots.lock().unwrap();
            slots.entry(key.to_owned()).or_default().queue.extend(items);
        }
    }

    fn bad() -> crate::providers::error::ProviderError {
        crate::providers::error::ProviderError {
            code: ProviderErrorCode::Unavailable,
            provider: ProviderId::Qq,
            message: "test fetch failure".to_owned(),
            retryable: false,
            action: None,
            raw_message: None,
        }
    }
}
