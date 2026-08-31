//! 轻量日志模块：等级过滤 + 控制台/文件双输出 + 按大小轮转。
//!
//! 日志文件保存在「程序启动目录」（可执行文件所在目录，获取失败回退当前工作目录）
//! 下的 `data/logs/` 目录：
//! ```text
//! <启动目录>/data/logs/app.log
//! <启动目录>/data/logs/app.log.1
//! <启动目录>/data/logs/app.log.2
//! ```
//! 单个文件超过 `max_file_size` 字节即轮转（`app.log` → `app.log.1` → …），
//! 最多保留 `max_files` 个轮转文件。
//!
//! 用法：
//! ```ignore
//! logger::init(logger::Config::default()).expect("初始化日志失败");
//! log::info!("hello {}", 42);
//! ```
//!
//! 业务代码只使用 `log` crate 的宏（`trace!`/`debug!`/`info!`/`warn!`/`error!`），
//! 不接触本模块细节。等级可通过 [`set_level`] 在运行时调整。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};

/// 业务日志等级（从低到高）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Level {
    /// 显示名称。
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }

    fn as_level_filter(self) -> LevelFilter {
        match self {
            Level::Error => LevelFilter::Error,
            Level::Warn => LevelFilter::Warn,
            Level::Info => LevelFilter::Info,
            Level::Debug => LevelFilter::Debug,
            Level::Trace => LevelFilter::Trace,
        }
    }

    /// 从字符串解析（大小写不敏感），供 RPC 命令使用。
    pub fn parse(s: &str) -> Option<Level> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Some(Level::Error),
            "warn" | "warning" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" => Some(Level::Debug),
            "trace" => Some(Level::Trace),
            _ => None,
        }
    }
}

/// 日志配置。
#[derive(Debug, Clone)]
pub struct Config {
    /// 打印等级：低于该等级的日志被过滤。
    pub level: Level,
    /// 是否同时输出到控制台（stderr）。
    pub console: bool,
    /// 单个日志文件最大字节数，超出即轮转。
    pub max_file_size: u64,
    /// 保留的轮转文件个数（`app.log.1` … `app.log.{max_files}`）。
    pub max_files: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            level: Level::Info,
            console: true,
            max_file_size: 5 * 1024 * 1024, // 5 MB
            max_files: 5,
        }
    }
}

/// 文件日志器：把 `log` 记录写入 `data/logs/` 并维护轮转。
struct Logger {
    config: Config,
    dir: PathBuf,
    /// 当前生效等级（可运行时调整）。
    level: Mutex<Level>,
    state: Mutex<State>,
}

struct State {
    file: Option<File>,
    size: u64,
}

impl Logger {
    fn new(config: Config, dir: PathBuf) -> Self {
        Self {
            level: Mutex::new(config.level),
            config,
            dir,
            state: Mutex::new(State { file: None, size: 0 }),
        }
    }

    fn set_level(&self, level: Level) {
        *self.level.lock().unwrap_or_else(|p| p.into_inner()) = level;
    }

    fn current_level(&self) -> Level {
        *self.level.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn log_file(&self) -> PathBuf {
        self.dir.join("app.log")
    }

    fn write_file(&self, line: &str) {
        let mut st = match self.state.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        if st.file.is_none() {
            st.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.log_file())
                .ok();
        }
        if let Some(f) = &mut st.file {
            let _ = f.write_all(line.as_bytes());
            let _ = f.write_all(b"\n");
            let _ = f.flush();
            st.size += line.len() as u64 + 1;
        }

        // 达到上限：关闭当前文件，轮转，并立即重建新的 app.log
        if st.size >= self.config.max_file_size {
            drop(st.file.take());
            st.size = 0;
            self.rotate();
            st.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.log_file())
                .ok();
        }
    }

    /// 按大小轮转：删除最老的 `app.log.{max_files}`，其余依次后移。
    fn rotate(&self) {
        let n = self.config.max_files;
        let last = self.dir.join(format!("app.log.{}", n));
        let _ = fs::remove_file(&last);

        for i in (1..=n).rev() {
            let from = if i == 1 {
                self.log_file()
            } else {
                self.dir.join(format!("app.log.{}", i - 1))
            };
            let to = self.dir.join(format!("app.log.{}", i));
            if from.exists() {
                let _ = fs::rename(&from, &to);
            }
        }
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.current_level().as_level_filter()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} [{}] {}: {}",
            crate::rpc::now_ms(),
            record.level().as_str(),
            record.target(),
            record.args()
        );
        if self.config.console {
            eprintln!("{line}");
        }
        self.write_file(&line);
    }

    fn flush(&self) {}
}

/// 是否已初始化（全局仅允许注册一次）。
static INITIALIZED: Mutex<bool> = Mutex::new(false);
/// 全局日志器实例（init 后借用）。
static LOGGER: Mutex<Option<&'static Logger>> = Mutex::new(None);

/// 初始化日志。应在程序入口最早调用；重复调用会被忽略。
pub fn init(config: Config) -> Result<(), String> {
    let mut guard = match INITIALIZED.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if *guard {
        return Ok(());
    }

    let dir = log_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("创建日志目录失败 {}: {e}", dir.display()))?;

    let logger: &'static Logger = Box::leak(Box::new(Logger::new(config.clone(), dir)));
    log::set_logger(logger).map_err(|e| format!("注册日志器失败: {e}"))?;
    *LOGGER.lock().unwrap_or_else(|p| p.into_inner()) = Some(logger);

    *guard = true;
    set_level(config.level);
    log::info!("日志已初始化: {}", log_dir().display());
    Ok(())
}

/// 运行时调整打印等级（业务/前端可调用），同步全局过滤与记录器内部过滤。
pub fn set_level(level: Level) {
    log::set_max_level(level.as_level_filter());
    if let Some(logger) = *LOGGER.lock().unwrap_or_else(|p| p.into_inner()) {
        logger.set_level(level);
    }
}

/// 当前生效的日志等级（未初始化时取全局过滤等级）。
pub fn current_level() -> Level {
    // LOGGER 未初始化时读记录器内部等级
    if let Some(logger) = *LOGGER.lock().unwrap_or_else(|p| p.into_inner()) {
        return logger.current_level();
    }
    match log::max_level() {
        log::LevelFilter::Error => Level::Error,
        log::LevelFilter::Warn => Level::Warn,
        log::LevelFilter::Info => Level::Info,
        log::LevelFilter::Debug => Level::Debug,
        log::LevelFilter::Trace => Level::Trace,
        log::LevelFilter::Off => Level::Info,
    }
}

/// 程序启动目录：优先可执行文件所在目录（安装/双击后稳定），获取失败回退当前工作目录。
fn startup_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// 日志目录：`<程序启动目录>/data/logs`。
pub fn log_dir() -> PathBuf {
    startup_dir().join("data").join("logs")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_parse_and_order() {
        assert_eq!(Level::parse("debug"), Some(Level::Debug));
        assert_eq!(Level::parse("WARN"), Some(Level::Warn));
        assert_eq!(Level::parse("warning"), Some(Level::Warn));
        assert_eq!(Level::parse("verbose"), None);
        assert!(Level::Debug > Level::Info);
    }

    #[test]
    fn log_dir_under_data_logs() {
        let dir = log_dir();
        assert_eq!(dir.file_name().unwrap().to_str(), Some("logs"));
        assert_eq!(dir.parent().unwrap().file_name().unwrap().to_str(), Some("data"));
    }

    #[test]
    fn rotation_truncates_and_bounds_files() {
        let dir = std::env::temp_dir().join("ccbuddy-log-rotate-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 单文件上限 100 字节，最多保留 3 个轮转文件
        let logger = Logger::new(
            Config {
                level: Level::Debug,
                console: false,
                max_file_size: 100,
                max_files: 3,
            },
            dir.clone(),
        );
        for i in 0..50 {
            logger.write_file(&format!("line {i}: xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"));
        }

        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        // 当前文件 + 最多 3 个轮转文件
        assert!(names.contains(&"app.log".to_string()));
        assert!(names.len() <= 4, "文件数超限: {names:?}");

        let _ = fs::remove_dir_all(&dir);
    }
}
