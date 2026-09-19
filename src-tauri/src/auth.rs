//! 服务端会话认证：密码校验 + 内存 session 表。
//!
//! 取代原先的 Basic Auth（浏览器原生弹框、凭据无过期语义）。当前方案：
//! - 登录走 `POST /api/login`，校验通过后签发随机 token，以 HttpOnly cookie 下发
//!   ——`EventSource` 无法设置请求头，cookie 是唯一能让 `fetch` 与 SSE 共用
//!   一套凭证的机制
//! - 空闲 [`IDLE_TIMEOUT`] 未发任何请求即失效；签发满 [`ABSOLUTE_TIMEOUT`] 强制失效
//! - **只存内存**：重启进程即全部失效，磁盘上不落任何 token
//!
//! 「活跃」由前端在标签页**可见时**发心跳产生（见 `src/auth.ts`），
//! 因此 [`Sessions::is_valid`] 刻意**不**续期——否则一个开着的事件流长连接
//! 会被持续推送一直续命，空闲过期形同虚设。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 空闲过期：距上次请求超过该时长即失效（15 分钟）。
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// 绝对过期：签发满该时长即失效，无论期间是否活跃（15 天）。
///
/// 同时用作 cookie 的 `Max-Age`（见 `server.rs::cookie_value`），两处必须一致。
pub const ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(15 * 24 * 60 * 60);

/// session cookie 名。全程只有这一个 cookie。
pub const COOKIE_NAME: &str = "ccbuddy_session";

/// 一个已签发的会话。
#[derive(Debug, Clone, Copy)]
struct Session {
    created: Instant,
    last_seen: Instant,
}

impl Session {
    fn expired(&self, now: Instant) -> bool {
        // duration_since 在参数更早时饱和到 0，不会 panic
        now.duration_since(self.created) >= ABSOLUTE_TIMEOUT
            || now.duration_since(self.last_seen) >= IDLE_TIMEOUT
    }
}

/// 内存 session 表（进程级单例，见 [`sessions`]）。
///
/// 带方法而非裸 `HashMap`：单测各自 `Sessions::new()` 建独立实例，不碰全局——
/// Rust 测试并行跑在同一个进程里，共用全局表会让过期边界用例互相干扰而 flake。
pub struct Sessions {
    map: Mutex<HashMap<String, Session>>,
}

impl Sessions {
    pub fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    /// 取锁：中毒（持锁线程 panic）时取回内部值继续用。
    /// 认证是全局路径，一次 panic 不该让之后每个请求都跟着 panic。
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Session>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 签发新 token（32 字节随机数的 hex，64 字符）。
    pub fn issue(&self) -> String {
        let token = random_token();
        let now = Instant::now();
        let mut map = self.lock();
        purge(&mut map, now);
        map.insert(
            token.clone(),
            Session {
                created: now,
                last_seen: now,
            },
        );
        token
    }

    /// 校验并**续期**（用户有实际请求）。有效返回 true。
    pub fn validate(&self, token: &str) -> bool {
        let now = Instant::now();
        let mut map = self.lock();
        purge(&mut map, now);
        match map.get_mut(token) {
            Some(s) => {
                s.last_seen = now;
                true
            }
            None => false,
        }
    }

    /// 校验但**不续期**。SSE 长连接存活与后台推送不该算「用户活跃」，
    /// 否则后台标签页会被推送一直续命，空闲过期永远不触发。
    pub fn is_valid(&self, token: &str) -> bool {
        let mut map = self.lock();
        purge(&mut map, Instant::now());
        map.contains_key(token)
    }

    /// 撤销单个 token（登出）。
    pub fn revoke(&self, token: &str) {
        self.lock().remove(token);
    }

    /// 撤销全部（改密码后强制所有浏览器重新登录）。
    pub fn revoke_all(&self) {
        self.lock().clear();
    }

    /// 当前有效会话数（单测断言用）。
    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }

    /// 单测用：塞一条指定时间戳的会话。
    /// `Instant` 无法凭空构造，只能从 `now` 往回拨。
    #[cfg(test)]
    fn insert_at(&self, token: &str, created: Instant, last_seen: Instant) {
        self.lock().insert(
            token.to_string(),
            Session { created, last_seen },
        );
    }
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程级 session 表。
///
/// 沿用 `server::sse_channel` 的 `OnceLock` 惯例：认证是全局状态，塞进
/// `AppState`（`Copy`，且 `sse_events` 刻意不取 extractor）要改动一串签名，不值当。
pub fn sessions() -> &'static Sessions {
    static SESSIONS: OnceLock<Sessions> = OnceLock::new();
    SESSIONS.get_or_init(Sessions::new)
}

/// 清掉已过期条目。
///
/// 每次签发/校验顺手全扫一遍——表里只有个位数条目，O(n) 可以忽略，
/// 换来的是**不需要任何定时器**（也就不用管定时器的生命周期与并发）。
fn purge(map: &mut HashMap<String, Session>, now: Instant) {
    map.retain(|_, s| !s.expired(now));
}

/// 32 字节随机数的 hex 串，用已有的 `rand_core::OsRng`（不引新依赖）。
fn random_token() -> String {
    use rand_core::{OsRng, RngCore};
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    let mut out = String::with_capacity(buf.len() * 2);
    for b in buf {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// `CCBUDDY_PASSWORD` 环境变量（非空才算设置）。
///
/// 它是**运行时覆盖**：优先级高于配置文件里的哈希，且不落盘。
/// 改密码命令据此拒绝执行——否则界面改完写进 config，下次启动环境变量又赢，
/// 改动静默失效、用户被关在门外（见 `core::handle` 的 `change_password`）。
pub fn env_password() -> Option<String> {
    std::env::var("CCBUDDY_PASSWORD").ok().filter(|p| !p.is_empty())
}

/// 校验登录密码。口径与 `config::server_password_or_init` 完全一致：
/// 环境变量优先（明文比较），否则比对配置里的 Argon2 哈希。
///
/// **每次调用都重新读**，不缓存启动时的快照——否则界面改了密码要重启才生效。
pub fn check_password(plain: &str) -> bool {
    if let Some(env) = env_password() {
        return constant_time_eq(plain, &env);
    }
    let stored = crate::config::stored_password_hash();
    !stored.is_empty() && crate::config::verify_password(plain, &stored)
}

/// 常量时间字符串比较（防时序侧信道）。
/// 长度不同直接返回 false——长度本身不是秘密，无需恒定时间。
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_hex_chars() {
        let t = random_token();
        assert_eq!(t.len(), 64);
        assert!(t.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        // 两次签发不应相同
        assert_ne!(t, random_token());
    }

    #[test]
    fn issue_validates_and_revoke_kills_it() {
        let s = Sessions::new();
        let t = s.issue();
        assert_eq!(s.len(), 1);
        assert!(s.validate(&t));
        assert!(s.is_valid(&t));

        s.revoke(&t);
        assert!(!s.validate(&t));
        assert_eq!(s.len(), 0);

        // 未签发过的 token 一律拒绝
        assert!(!s.validate("deadbeef"));
        assert!(!s.validate(""));
    }

    #[test]
    fn revoke_all_clears_everything() {
        let s = Sessions::new();
        let a = s.issue();
        let b = s.issue();
        assert_eq!(s.len(), 2);

        s.revoke_all();
        assert!(!s.validate(&a));
        assert!(!s.validate(&b));
        assert_eq!(s.len(), 0);
    }

    /// 两条过期线各自的边界：差一点仍有效，刚好到即失效。
    #[test]
    fn idle_and_absolute_expiry_boundaries() {
        let s = Sessions::new();
        let now = Instant::now();

        // 空闲差 1 秒到上限：仍有效
        s.insert_at("idle-edge-ok", now, now - IDLE_TIMEOUT + Duration::from_secs(1));
        assert!(s.validate("idle-edge-ok"));

        // 空闲刚好到上限：失效
        s.insert_at("idle-edge-out", now, now - IDLE_TIMEOUT);
        assert!(!s.validate("idle-edge-out"));

        // 空闲未超，但签发已满绝对上限：失效
        s.insert_at("abs-out", now - ABSOLUTE_TIMEOUT, now);
        assert!(!s.validate("abs-out"));

        // 签发差 1 秒到绝对上限、且刚活跃过：仍有效
        s.insert_at(
            "abs-edge-ok",
            now - ABSOLUTE_TIMEOUT + Duration::from_secs(1),
            now,
        );
        assert!(s.validate("abs-edge-ok"));
    }

    /// 空闲过期的条目在下一次签发/校验时被顺手清掉，表不会无限增长。
    #[test]
    fn purge_drops_expired_entries() {
        let s = Sessions::new();
        let now = Instant::now();
        s.insert_at("stale", now - IDLE_TIMEOUT, now - IDLE_TIMEOUT);
        assert_eq!(s.len(), 1);

        // 任何一次签发都会触发全表清理
        let fresh = s.issue();
        assert_eq!(s.len(), 1);
        assert!(s.is_valid(&fresh));
    }

    /// 续期确实生效：validate 会推开空闲计时，is_valid 不会。
    #[test]
    fn validate_refreshes_idle_but_is_valid_does_not() {
        let s = Sessions::new();
        let now = Instant::now();

        // 距空闲上限还剩 10 秒
        s.insert_at("renewed", now, now - IDLE_TIMEOUT + Duration::from_secs(10));
        assert!(s.validate("renewed"));

        // 续期后 last_seen 被推到当下，再查仍是有效
        assert!(s.is_valid("renewed"));

        // is_valid 不续期：再塞一条濒临过期的，连续查两次状态不变
        s.insert_at("no-renew", now, now - IDLE_TIMEOUT + Duration::from_secs(1));
        assert!(s.is_valid("no-renew"));
        assert!(s.is_valid("no-renew"));
    }

    #[test]
    fn constant_time_eq_matches_equality() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "abcd"));
        assert!(constant_time_eq("", ""));
        // 非 ASCII（多字节 UTF-8）按字节比较
        assert!(constant_time_eq("密码", "密码"));
        assert!(!constant_time_eq("密码", "密玛"));
    }
}
