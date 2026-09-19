//! 应用配置：`~/.ccbuddy/config.json`。
//!
//! 集中管理用户可配置项（如 Claude 目录位置），桌面端与 server 共用。
//! 约束：本配置描述的是「读取位置」，程序对 `~/.claude` 目录只读不写
//! （hook 安装除外，见 install_hooks）。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// 用户配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Claude Code 数据目录（默认 `~/.claude`）。
    /// 用于读取 projects/ 下的历史会话 transcript 与 settings.json。
    #[serde(default = "default_claude_dir")]
    pub claude_dir: String,
    /// 用量自动刷新周期（秒）。后端按此间隔检查 transcript 变化并 SSE 推送。
    /// 钳制到 [5, 3600]，默认 30。
    #[serde(default = "default_usage_refresh_secs")]
    pub usage_refresh_secs: u32,
    /// ccbuddy-server 监听配置（端口 + 访问密码）。
    #[serde(default)]
    pub server: ServerConfig,
}

/// server 监听配置（`~/.ccbuddy/config.json` 的 `server` 段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 监听地址（默认 127.0.0.1 仅本机；对外服务改为 0.0.0.0）。
    #[serde(default = "default_server_bind")]
    pub bind: String,
    /// 监听端口（默认 18787）。命令行 `-p` 参数可临时覆盖。
    #[serde(default = "default_server_port")]
    pub port: u16,
    /// 访问密码（server 强制启用验证，本字段不存明文）。
    ///
    /// 本字段存 **Argon2id 哈希**（`$argon2id$...`）：
    /// - 空值（首次启动）→ [`server_password_or_init`] 自动生成随机密码
    ///   哈希写回本字段，明文仅终端打印一次
    /// - 设置/修改密码：`ccbuddy-server --set-password`（交互输入，哈希落盘）
    /// - 清空：`--clear-password`（下次启动重新生成随机密码）
    /// 临时覆盖：CCBUDDY_PASSWORD 环境变量（明文，仅进程内存，不落盘）。
    #[serde(default)]
    pub password: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_server_bind(),
            port: default_server_port(),
            password: String::new(),
        }
    }
}

fn default_server_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_server_port() -> u16 {
    18787
}

impl Default for Config {
    fn default() -> Self {
        Self {
            claude_dir: default_claude_dir(),
            usage_refresh_secs: default_usage_refresh_secs(),
            server: ServerConfig::default(),
        }
    }
}

fn default_claude_dir() -> String {
    dirs::home_dir()
        .map(|h| h.join(".claude").to_string_lossy().to_string())
        .unwrap_or_else(|| ".claude".to_string())
}

/// 用量刷新周期下限/上限（秒）：过小浪费 CPU，过大失去"实时"意义。
pub const USAGE_REFRESH_MIN: u32 = 5;
pub const USAGE_REFRESH_MAX: u32 = 3600;

fn default_usage_refresh_secs() -> u32 {
    30
}

/// 钳制刷新周期到合法区间（写入与读取两端都过一遍，老配置脏值兜底）。
pub fn clamp_usage_refresh_secs(secs: u32) -> u32 {
    secs.clamp(USAGE_REFRESH_MIN, USAGE_REFRESH_MAX)
}

/// ccbuddy 数据根目录：`~/.ccbuddy`。
///
/// 环境变量 `CCBUDDY_DATA_ROOT` 可覆盖（测试隔离 / 便携部署用）；
/// 未设置或为空串时按默认 `~/.ccbuddy`，与历史行为完全一致。
pub fn data_root() -> PathBuf {
    if let Ok(dir) = std::env::var("CCBUDDY_DATA_ROOT") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ccbuddy")
}

/// 配置文件路径：`~/.ccbuddy/config.json`。
pub fn config_path() -> PathBuf {
    data_root().join("config.json")
}

/// 事件流日志目录：`~/.ccbuddy/events`。
///
/// hook 往里追加 `event-<会话 id>.jsonl`，前端读它看实时会话。
/// 放在 config 里而不是 events_mgr 里：路径由数据根目录派生，
/// 属于「配置」这一层；否则 config 视图要反过来调 events_mgr，形成环路。
pub fn events_dir() -> PathBuf {
    data_root().join("events")
}

/// 读取配置；文件不存在或损坏时返回默认值（不写盘）。
pub fn load() -> Config {
    // 读取失败（不存在/权限）或 JSON 损坏：都回退默认配置，不报错
    let text: Option<String> = std::fs::read_to_string(config_path()).ok();
    let parsed: Option<Config> = text.and_then(|t| serde_json::from_str(&t).ok());
    parsed.unwrap_or_default()
}

/// 保存配置（pretty JSON）。
pub fn save(cfg: &Config) -> Result<(), String> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("写入配置失败: {e}"))
}

/// 解析配置中的 Claude 目录（claude_dir 为空时用默认值）。
pub fn claude_dir() -> PathBuf {
    let cfg = load();
    let dir = cfg.claude_dir.trim();
    if dir.is_empty() {
        PathBuf::from(default_claude_dir())
    } else {
        PathBuf::from(dir)
    }
}

/// 生成密码的 Argon2id 哈希（带随机盐），用于写入配置文件。
/// 算法参数走 Argon2 默认（m=19456KB, t=2, p=1），安全且校验开销 <100ms。
pub fn hash_password(plain: &str) -> Result<String, String> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    use rand_core::OsRng;

    let salt = SaltString::generate(&mut OsRng);
    argon2::Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("密码哈希失败: {e}"))
}

/// 校验明文密码是否与存储的 Argon2 哈希匹配。
pub fn verify_password(plain: &str, stored: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    use argon2::Argon2;
    // 哈希串解析失败（格式损坏）或校验失败：一律拒绝
    match PasswordHash::new(stored) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// 生成 16 位随机密码（去易混淆字符 0/O/1/l/I），首次启动兜底用。
fn random_password() -> String {
    use rand_core::{OsRng, RngCore};
    const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz23456789";
    let mut buf = [0u8; 16];
    OsRng.fill_bytes(&mut buf);
    buf.iter()
        .map(|b| CHARSET[(*b as usize) % CHARSET.len()] as char)
        .collect()
}

/// 取 server 访问密码（校验器语义），**取不到就生成**（强制验证，不裸奔）：
/// - 设了 CCBUDDY_PASSWORD → (明文标记, 值)，无首次明文
/// - 配置已有值 → 按存储形态（哈希/明文）返回，无首次明文
/// - 都没有 → 生成 16 位随机密码，Argon2 哈希写盘，
///   返回 (哈希标记, 值) + 首次明文（仅供启动终端打印一次，不落盘）
///
/// 返回 ((is_hash, value), initial_plain)：is_hash 时校验走 Argon2，否则明文比较。
pub fn server_password_or_init() -> ((bool, String), Option<String>) {
    // 环境变量优先：明文，进程内存中，不落盘
    if let Ok(p) = std::env::var("CCBUDDY_PASSWORD") {
        if !p.is_empty() {
            return ((false, p), None);
        }
    }
    let mut cfg = load();
    if !cfg.server.password.is_empty() {
        return ((cfg.server.password.starts_with("$argon2"), cfg.server.password), None);
    }
    // 首次启动：生成随机密码并哈希落盘
    let plain = random_password();
    match hash_password(&plain) {
        Ok(hashed) => {
            cfg.server.password = hashed.clone();
            let saved = save(&cfg).is_ok();
            if !saved {
                log::warn!("密码哈希写入配置文件失败，本次运行用内存中的哈希，重启将重新生成");
            }
            ((true, hashed), Some(plain))
        }
        // 哈希失败（极小概率）：内存明文兜底，保证验证仍然启用
        Err(_) => ((false, plain.clone()), Some(plain)),
    }
}

/// 写入 server 访问密码（Some = 哈希值，None = 清空）。`--set-password` 用。
pub fn write_server_password(hashed: Option<&str>) -> Result<(), String> {
    let mut cfg = load();
    cfg.server.password = hashed.unwrap_or("").to_string();
    save(&cfg)
}

/// 配置里存储的密码哈希（可能是空串 = 尚未设置）。
///
/// 与 [`server_password_or_init`] 的区别：**只读，不带「没有就生成」的副作用**。
/// 登录校验每次现读，才能让界面改完密码立即生效（不必重启进程）。
pub fn stored_password_hash() -> String {
    load().server.password
}

/// server 监听地址（含端口）的配置取值：`bind:port` 都来自配置文件
/// （未配置时默认 127.0.0.1:18787）。`-p` 参数由入口解析后覆盖端口。
///
/// 安全提示：bind 改为 0.0.0.0 对外服务时，务必同时设置 password。
pub fn server_addr() -> String {
    let cfg = load();
    let bind = {
        let b = cfg.server.bind.trim();
        if b.is_empty() { default_server_bind() } else { b.to_string() }
    };
    format!("{bind}:{}", cfg.server.port)
}

/// 前端可见的配置视图：用户配置 + 只读派生字段（日志源目录等）。
///
/// 只读字段不入盘，每次由当前状态派生，避免前后端各查一遍。
pub fn config_view() -> ConfigView {
    let cfg = load();
    ConfigView {
        claude_dir: cfg.claude_dir,
        usage_refresh_secs: clamp_usage_refresh_secs(cfg.usage_refresh_secs),
        // 只读派生字段。日志源目录与数据根目录同源（都从 data_root 派生），
        // 因此这里只依赖 config 自己，不反向依赖 events_mgr。
        events_dir: events_dir().to_string_lossy().to_string(),
        data_root: data_root().to_string_lossy().to_string(),
        log_level: crate::logger::current_level().as_str().to_string(),
    }
}

/// 配置视图（get_config 返回 / set_config 回显）。
#[derive(Debug, Clone, Serialize)]
pub struct ConfigView {
    /// Claude Code 数据目录（空串 = 默认 ~/.claude）
    pub claude_dir: String,
    /// 用量自动刷新周期（秒，钳制 [5, 3600]）
    pub usage_refresh_secs: u32,
    /// 日志源目录（只读）
    pub events_dir: String,
    /// ccbuddy 数据根目录（只读）
    pub data_root: String,
    /// 当前日志等级（只读快照，修改走 log_level 字段）
    pub log_level: String,
}

/// 部分更新配置：只覆盖 patch 中出现的字段，未知字段报错。
///
/// 可写字段：claude_dir / log_level（运行时项，不入盘）。
pub fn apply_patch(patch: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let mut cfg = load();

    for (key, val) in patch {
        let text = val
            .as_str()
            .ok_or_else(|| format!("字段 {key} 需为字符串"))?;
        match key.as_str() {
            "claude_dir" => {
                cfg.claude_dir = text.trim().to_string();
                log::info!("Claude 目录已设置为: {}", if cfg.claude_dir.is_empty() { "(默认 ~/.claude)" } else { &cfg.claude_dir });
            }
            // 用量刷新周期：钳制到合法区间后写盘，watcher 下一轮读取生效
            "usage_refresh_secs" => {
                let secs: u32 = text
                    .trim()
                    .parse()
                    .map_err(|_| format!("刷新周期需为数字: {text}"))?;
                let clamped = clamp_usage_refresh_secs(secs);
                cfg.usage_refresh_secs = clamped;
                log::info!("用量刷新周期已设置为 {clamped} 秒");
            }
            // 运行时日志等级：立即生效，不持久化
            "log_level" => {
                let level = crate::logger::Level::parse(text)
                    .ok_or_else(|| format!("无效日志等级: {text}"))?;
                crate::logger::set_level(level);
                log::info!("日志等级已设置为 {}", level.as_str());
            }
            _ => return Err(format!("未知或只读字段: {key}")),
        }
    }

    // 有持久化字段变化时才写盘
    if patch.keys().any(|k| k == "claude_dir" || k == "usage_refresh_secs") {
        save(&cfg)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_roundtrip_and_verify() {
        let hashed = hash_password("s3cret-Pass").expect("哈希生成失败");
        // 格式：PHC 字符串 $argon2id$...
        assert!(hashed.starts_with("$argon2id$"), "应为 argon2id PHC 格式: {hashed}");
        assert!(!hashed.contains("s3cret-Pass"), "哈希中不得包含明文");
        // 正确密码通过，错误密码拒绝
        assert!(verify_password("s3cret-Pass", &hashed));
        assert!(!verify_password("wrong", &hashed));
    }

    #[test]
    fn hash_has_random_salt() {
        // 每次哈希带随机盐：同一明文两次结果不同
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b, "同明文两次哈希应不同（随机盐）");
    }

    #[test]
    fn corrupted_hash_rejects() {
        // 哈希格式损坏：拒绝而不是放行
        assert!(!verify_password("any", "$argon2id$garbage"));
    }
}
