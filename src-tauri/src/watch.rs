//! 通用「后台定时推送」件。
//!
//! 事件流（[`crate::events_mgr`]）与用量（[`crate::usage_mgr`]）各有一条同构的
//! 推送链路：注册一批回调 → 后台线程按周期取数 → 回调全部监听者。
//! 两条链路的差异只有两处，都由调用方以参数给出：
//!
//! | 差异 | 事件流 | 用量 |
//! |---|---|---|
//! | 推送周期 | 固定 2s | 来自配置（默认 30s，可热改） |
//! | 取数内容 | 事件流会话列表 | 用量记录列表 |
//!
//! **每轮都推**（不做「内容没变就不推」的抑制）是刻意的：卡片 / 趋势 /
//! 活跃热力图这类视图的「今天」分桶、相对时间都随墙上时钟变化，只在数据变化时
//! 推会让界面一直停在旧值（用户看到的「不会定时刷新」）。取数侧本身有 mtime
//! 增量缓存（见 [`crate::events_mgr`] / [`crate::usage_mgr`]），文件没变时不会
//! 重复解析，空转轮次的成本只剩序列化与投递。
//!
//! 不引第三方 crate：推送由本模块的定时线程驱动，不依赖文件系统事件通知。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// 周期函数：每轮重新求值，便于配置热改后下一轮生效。
pub type Period = fn() -> Duration;

/// 监听者注册表：按自增稳定 id 管理回调。
///
/// id 单调递增且永不复用，注销时按 id 移除——若按下标 `remove`，
/// 并发注销会在中间元素被移除后整体前移，误删别的监听者。
///
/// 只对回调类型 `F` 泛型；投递数据的类型由 [`Self::notify`] 就地给出，
/// 注册表本身无需关心（谁的数据谁自己传）。
pub struct ListenerRegistry<F: Send + 'static> {
    entries: Mutex<Vec<Entry<F>>>,
    next_id: AtomicU64,
}

/// 注册表中的一项：稳定 id + 回调。
struct Entry<F> {
    id: u64,
    f: F,
}

impl<F: Send + 'static> ListenerRegistry<F> {
    /// 常量构造，供 `static` 注册表使用。
    pub const fn new() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// 注册回调，返回注销凭据（drop 即注销）。
    ///
    /// 取 `&'static self`：注册表本身是进程级 `static`，
    /// 凭据因此不需要生命周期参数，调用方也不用持有注册表的所有权。
    pub fn subscribe(&'static self, f: F) -> ListenerGuard<F> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.entries.lock().unwrap().push(Entry { id, f });
        ListenerGuard {
            registry: self,
            id,
        }
    }

    /// 按注册顺序回调全部监听者，传入各自需要的参数 `arg`。
    ///
    /// 持锁回调（与历史实现一致）：回调里不得再进本注册表，否则死锁。
    pub fn notify<A: Clone>(&self, arg: A)
    where
        F: Fn(A),
    {
        for e in self.entries.lock().unwrap().iter() {
            (e.f)(arg.clone());
        }
    }

    /// 按稳定 id 移除。
    fn unsubscribe(&self, id: u64) {
        self.entries.lock().unwrap().retain(|e| e.id != id);
    }
}

/// 注销凭据：drop 时把自己从注册表移除（凭据遗失即自动注销）。
pub struct ListenerGuard<F: Send + 'static> {
    registry: &'static ListenerRegistry<F>,
    id: u64,
}

impl<F: Send + 'static> Drop for ListenerGuard<F> {
    fn drop(&mut self) {
        self.registry.unsubscribe(self.id);
    }
}

/// 单次启动的定时推送器：一个用途一个 `static`（各自独立的一次性开关）。
///
/// 用 `OnceLock` 保证只起一个线程：桌面端与 server 同时运行时，
/// `subscribe` 会被调两次，但推送线程只能有一个（否则同一份数据被推两遍）。
pub struct PollingWatcher {
    started: OnceLock<()>,
}

impl PollingWatcher {
    /// 常量构造，供 `static` 使用。
    pub const fn new() -> Self {
        Self {
            started: OnceLock::new(),
        }
    }

    /// 启动推送线程。重复调用直接返回（幂等）。
    ///
    /// 线程内每轮：`sleep(period)` → `on_tick`（**不判断数据是否变化**，
    /// 理由见模块文档）。首个回调在第一个周期之后触发；前端首屏数据由 RPC
    /// 调用与 SSE 建连首帧负责，不依赖这里的首轮。
    pub fn start(&'static self, period: Period, on_tick: impl Fn() + Send + 'static) {
        if self.started.set(()).is_err() {
            return; // 已启动
        }
        std::thread::spawn(move || loop {
            std::thread::sleep(period());
            on_tick();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 注册 / 注销：注销一个不影响另一个（L-B14 的回归测试）。
    #[test]
    fn listener_unregister_does_not_affect_others() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        use std::sync::Arc;

        static REG: ListenerRegistry<Box<dyn Fn(u32) + Send + Sync>> =
            ListenerRegistry::new();

        let a_calls = Arc::new(AtomicUsize::new(0));
        let b_calls = Arc::new(AtomicUsize::new(0));
        let ga = {
            let a = a_calls.clone();
            REG.subscribe(Box::new(move |_| {
                a.fetch_add(1, AtomicOrdering::SeqCst);
            }))
        };
        let gb = {
            let b = b_calls.clone();
            REG.subscribe(Box::new(move |_| {
                b.fetch_add(1, AtomicOrdering::SeqCst);
            }))
        };

        drop(ga); // 注销 A：按下标移除的旧实现会误删 B
        REG.notify(1);
        assert_eq!(a_calls.load(AtomicOrdering::SeqCst), 0, "已注销者不应再被回调");
        assert_eq!(b_calls.load(AtomicOrdering::SeqCst), 1, "其余监听者应存活");

        drop(gb);
        REG.notify(1);
        assert_eq!(b_calls.load(AtomicOrdering::SeqCst), 1, "全部注销后无人被回调");
    }

    /// 回调收到的数据与注册顺序一致（notify 逐个投递同一份数据的克隆）。
    #[test]
    fn notify_delivers_to_all_in_order() {
        use std::sync::Mutex as StdMutex;
        static REG: ListenerRegistry<Box<dyn Fn(Vec<u32>) + Send + Sync>> =
            ListenerRegistry::new();
        static SEEN: StdMutex<Vec<Vec<u32>>> = StdMutex::new(Vec::new());

        let g1 = REG.subscribe(Box::new(|v| SEEN.lock().unwrap().push(v)));
        REG.notify(vec![7, 8]);
        assert_eq!(*SEEN.lock().unwrap(), vec![vec![7, 8]]);
        drop(g1);
        SEEN.lock().unwrap().clear();
        REG.notify(vec![9]);
        assert!(SEEN.lock().unwrap().is_empty(), "注销后不再投递");
    }
}
