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
    /// GitHub 仓库（`owner/repo`），自动下载 hook 时从该仓库的 latest release 拉取。
    #[serde(default = "default_github_repo")]
    pub github_repo: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            claude_dir: default_claude_dir(),
            github_repo: default_github_repo(),
        }
    }
}

fn default_claude_dir() -> String {
    dirs::home_dir()
        .map(|h| h.join(".claude").to_string_lossy().to_string())
        .unwrap_or_else(|| ".claude".to_string())
}

fn default_github_repo() -> String {
    "xutopia77/ccbuddy".to_string()
}

/// ccbuddy 数据根目录：`~/.ccbuddy`。
pub fn data_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ccbuddy")
}

/// 配置文件路径：`~/.ccbuddy/config.json`。
pub fn config_path() -> PathBuf {
    data_root().join("config.json")
}

/// 读取配置；文件不存在或损坏时返回默认值（不写盘）。
pub fn load() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
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

/// 前端可见的配置视图：用户配置 + 只读派生字段（日志源目录等）。
///
/// 只读字段不入盘，每次由当前状态派生，避免前后端各查一遍。
pub fn config_view() -> ConfigView {
    let cfg = load();
    ConfigView {
        claude_dir: cfg.claude_dir,
        github_repo: cfg.github_repo,
        // 只读派生字段
        events_dir: crate::state::events_dir().to_string_lossy().to_string(),
        data_root: data_root().to_string_lossy().to_string(),
        log_level: crate::logger::current_level().as_str().to_string(),
    }
}

/// 配置视图（get_config 返回 / set_config 回显）。
#[derive(Debug, Clone, Serialize)]
pub struct ConfigView {
    /// Claude Code 数据目录（空串 = 默认 ~/.claude）
    pub claude_dir: String,
    /// GitHub 仓库（hook 自动下载源）
    pub github_repo: String,
    /// 日志源目录（只读）
    pub events_dir: String,
    /// ccbuddy 数据根目录（只读）
    pub data_root: String,
    /// 当前日志等级（只读快照，修改走 log_level 字段）
    pub log_level: String,
}

/// 部分更新配置：只覆盖 patch 中出现的字段，未知字段报错。
///
/// 可写字段：claude_dir / github_repo / log_level（运行时项，不入盘）。
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
            "github_repo" => {
                cfg.github_repo = text.trim().to_string();
                log::info!("GitHub 仓库已设置为: {}", if cfg.github_repo.is_empty() { "(默认 xutopia77/ccbuddy)" } else { &cfg.github_repo });
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
    if patch.keys().any(|k| k == "claude_dir" || k == "github_repo") {
        save(&cfg)?;
    }
    Ok(())
}
