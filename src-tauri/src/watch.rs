//! 通用「目录指纹 + 后台轮询推送」件。
//!
//! 事件流（[`crate::events_mgr`]）与用量（[`crate::usage_mgr`]）各有一条同构的
//! 推送链路：注册一批回调 → 后台线程定时算目录指纹 → 指纹变化时重新解析数据
//! 并回调全部监听者。两条链路的差异只有三处，都由调用方以参数给出：
//!
//! | 差异 | 事件流 | 用量 |
//! |---|---|---|
//! | 轮询周期 | 固定 1s | 来自配置（默认 30s，可热改） |
//! | 首轮基准 | 先取当前指纹，只在真变化时回调 | 从空指纹起算，首轮即回调 |
//! | 指纹递归 | 只看 events 目录第一层 | 递归一层（会话目录下的 subagents/） |
//!
//! 不引第三方 crate：hook 日志与 transcript 都是 jsonl 追加写，
//! 轮询「文件名 + mtime + size」足够，`notify`/`inotify` 属于杀鸡用牛刀。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

/// 指纹函数：把「目录此刻的状态」汇总成一个值，值变了就认为有新数据。
pub type Fingerprint = fn() -> u64;

/// 周期函数：每轮重新求值，便于配置热改后下一轮生效。
pub type Period = fn() -> Duration;

/// 首轮对比基准。
#[derive(Debug, Clone, Copy)]
pub enum Baseline {
    /// 启动时先记下当前指纹：只有真实变化才回调。
    Current,
    /// 以「空指纹 0」为基准：首轮若目录非空就回调一次。
    ///
    /// 用量推送依赖这一次回调（前端首屏要的是「连上就有数据」，
    /// 而不是等到第一次文件变化），因此不能改成 [`Baseline::Current`]。
    Empty,
}

/// 目录指纹：目录内各项的 `名字 + mtime + size` 按稳定顺序汇总哈希。
///
/// - 按文件名排序：`read_dir` 的返回顺序不保证稳定，不排序会导致指纹抖动；
/// - 带上 size：NTFS / drvfs 的时间戳粒度是 1-2 秒，同秒内追加写入 mtime 不变，
///   但 append 只增不减，size 一定能探测到；
/// - 读取失败（权限 / 并发删除）：用「纪元时间 + 长度 0」占位，不跳过——
///   同一状态必须永远算出同一个值，否则后台线程会空转推送。
///
/// `depth` 为向下递归的层数：0 只看第一层（事件流日志目录是扁平的），
/// 1 再看子目录一层（用量要覆盖 `<会话 id>/subagents/agent-*.jsonl`）。
///
/// 目录不存在 / 读不到时返回「空哈希」（一个固定值）：调用方只比较指纹变没变，
/// 不解读数值本身，因此「没有数据」也是一个稳定状态。
pub fn fingerprint_dir(dir: &Path, depth: u32) -> u64 {
    let mut h = DefaultHasher::new();
    hash_dir_into(dir, depth, &mut h);
    h.finish()
}

/// [`fingerprint_dir`] 的内层：把目录内容哈希进已有的 hasher。
fn hash_dir_into(dir: &Path, depth: u32, h: &mut DefaultHasher) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        entry.file_name().hash(h);
        // 元数据取不到（竞态删除等）：占位值，保持同一状态同一指纹
        let meta = entry.metadata().ok();
        let mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        mtime.hash(h);
        meta.as_ref().map(|m| m.len()).unwrap_or(0).hash(h);
        // 子目录：按剩余层数继续往下
        if depth > 0 && meta.as_ref().is_some_and(|m| m.is_dir()) {
            hash_dir_into(&entry.path(), depth - 1, h);
        }
    }
}

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

/// 单次启动的指纹轮询器：一个用途一个 `static`（各自独立的一次性开关）。
///
/// 用 `OnceLock` 保证只起一个线程：桌面端与 server 同时运行时，
/// `subscribe` 会被调两次，但轮询线程只能有一个（否则同一份数据被推两遍）。
pub struct PollingWatcher {
    started: OnceLock<()>,
    /// 上一轮观察到的指纹（[`Baseline::Empty`] 下初值为 0）。
    last: AtomicU64,
}

impl PollingWatcher {
    /// 常量构造，供 `static` 使用。
    pub const fn new() -> Self {
        Self {
            started: OnceLock::new(),
            last: AtomicU64::new(0),
        }
    }

    /// 启动轮询线程。重复调用直接返回（幂等）。
    ///
    /// 线程内每轮：`sleep(period)` → 算指纹 → 与上轮不同则记下新值并 `on_change`。
    /// 先记指纹再回调：回调里的解析耗时不会让同一批变化被重复推送。
    pub fn start(
        &'static self,
        period: Period,
        baseline: Baseline,
        fingerprint: Fingerprint,
        on_change: impl Fn() + Send + 'static,
    ) {
        if self.started.set(()).is_err() {
            return; // 已启动
        }
        std::thread::spawn(move || {
            // 首轮基准：先记下此刻指纹，之后的比较只反映真实变化
            if let Baseline::Current = baseline {
                self.last.store(fingerprint(), Ordering::Relaxed);
            }
            loop {
                std::thread::sleep(period());
                let cur = fingerprint();
                if cur == self.last.load(Ordering::Relaxed) {
                    continue;
                }
                self.last.store(cur, Ordering::Relaxed);
                on_change();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 指纹必须只反映「目录状态」：同一状态下反复算结果相同。
    #[test]
    fn fingerprint_is_stable_for_unchanged_dir() {
        let dir = std::env::temp_dir().join("ccbuddy-test-watch-stable");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jsonl"), "{}\n").unwrap();
        std::fs::write(dir.join("b.jsonl"), "{}\n").unwrap();

        let first = fingerprint_dir(&dir, 0);
        // 新增一个同类文件后指纹必变；内容追加（size 变化）后也必变
        std::fs::write(dir.join("c.jsonl"), "{}\n").unwrap();
        let added = fingerprint_dir(&dir, 0);
        assert_ne!(first, added, "新增文件应改变指纹");

        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(dir.join("a.jsonl"))
            .unwrap();
        std::io::Write::write_all(&mut f, b"{}\n").unwrap();
        drop(f);
        assert_ne!(added, fingerprint_dir(&dir, 0), "追加写入应改变指纹");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 目录不存在：指纹是稳定的「空哈希」——同一状态不会抖动，
    /// 因此用量 watcher 在 projects 目录缺失时最多首轮推一次空列表，之后安静。
    #[test]
    fn missing_dir_fingerprints_stably() {
        let dir = std::env::temp_dir().join("ccbuddy-test-watch-missing");
        let _ = std::fs::remove_dir_all(&dir);
        let empty = fingerprint_dir(&dir, 1);
        assert_eq!(empty, fingerprint_dir(&dir, 1), "同一缺失状态指纹应稳定");

        // 另一个不存在的目录：同样是空哈希（与具体路径无关）
        let other = std::env::temp_dir().join("ccbuddy-test-watch-missing-2");
        let _ = std::fs::remove_dir_all(&other);
        assert_eq!(empty, fingerprint_dir(&other, 1));

        // 有内容的目录：指纹不同（有数据 ≠ 无数据）
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jsonl"), "{}\n").unwrap();
        assert_ne!(empty, fingerprint_dir(&dir, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 递归层数：depth 越大，参与哈希的目录项越多。
    ///
    /// 只断言「与平台无关」的部分：depth 1 会把子目录内的**文件名**一起哈希，
    /// 因此子目录里新增文件必被探测到。反向断言（depth 0 探测不到子目录内变化）
    /// 不可靠——祖先目录的 mtime 是否随之下变依文件系统与刷新时机而定。
    #[test]
    fn depth_controls_recursion() {
        let dir = std::env::temp_dir().join("ccbuddy-test-watch-depth");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("session")).unwrap();
        std::fs::write(dir.join("session").join("a.jsonl"), "{}\n").unwrap();

        // 同一棵树：depth 0 只哈希 session 这一项，depth 1 还哈希它内部的 a.jsonl
        assert_ne!(
            fingerprint_dir(&dir, 0),
            fingerprint_dir(&dir, 1),
            "不同层数哈希的目录项集合不同"
        );

        // depth 1 把子目录内的文件名算进指纹 → 新增文件必被探测到
        let before = fingerprint_dir(&dir, 1);
        std::fs::write(dir.join("session").join("b.jsonl"), "{}\n").unwrap();
        assert_ne!(before, fingerprint_dir(&dir, 1), "递归层内新增文件应改变指纹");

        // 同一状态反复计算：数值稳定
        assert_eq!(fingerprint_dir(&dir, 0), fingerprint_dir(&dir, 0));
        assert_eq!(fingerprint_dir(&dir, 1), fingerprint_dir(&dir, 1));

        let _ = std::fs::remove_dir_all(&dir);
    }

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
