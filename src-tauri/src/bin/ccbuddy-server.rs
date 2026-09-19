//! ccbuddy-server：无头服务端入口（无桌面环境的 Linux 服务器）。
//!
//! 启动内嵌 HTTP 服务，托管与桌面端完全一致的 Vue 前端（编译产物嵌入二进制），
//! 浏览器访问即可查看 Claude Code 会话事件流。
//!
//! ## 监听配置
//! 地址与端口默认取配置文件 `~/.ccbuddy/config.json` 的 `server` 段
//! （bind 默认 127.0.0.1，port 默认 18787）；`-p <端口>` 参数临时覆盖端口。
//!
//! ## 访问密码（多用户主机必读）
//! - **强制验证**：server 必定启用密码，不存在无密码裸奔状态
//! - 首次启动：自动生成 16 位随机密码，终端打印一次（请立即改掉），
//!   Argon2 哈希写入配置文件，明文不落盘
//! - 登录：浏览器打开后是应用自己的登录页（**不是**浏览器原生 Basic Auth 弹框），
//!   登录成功签发 session token（HttpOnly cookie，空闲 15 分钟 / 最长 15 天）。
//!   登录态只存内存，重启服务即需重新登录
//! - 界面内改密码：设置页「修改密码」（改完所有浏览器需重新登录）
//! - 忘记密码：`--clear-password` 清空后重启，会重新生成随机密码并打印
//!   （也可直接删掉配置文件里的 `password` 字段）
//! - 离线改密码：`--set-password` 交互输入（哈希存储）
//! - 临时覆盖：CCBUDDY_PASSWORD 环境变量（明文，仅进程内存；设置后界面内改密码会被拒绝）
//!
//! 用法：
//!   ccbuddy-server                  # 配置文件为准（默认 127.0.0.1:18787）
//!   ccbuddy-server -p 9000          # 本次运行覆盖端口
//!   ccbuddy-server --set-password   # 设置/修改访问密码
//!   ccbuddy-server --clear-password # 清空密码（下次启动重新生成随机密码）
//!   ccbuddy-server -h               # 帮助
//!
//! 构建时要求前端产物 ../dist 已存在（打包脚本会先执行 npm run build）。

use include_dir::{include_dir, Dir};

// '_ lifetime 是一个生命周期标注，表示这个 Dir 的生命周期与包含它的作用域相同。由于 include_dir! 宏在编译时就将目录内容嵌入到二进制中，因此这个 Dir 的生命周期是静态的，整个程序运行期间都有效。
// $CARGO_MANIFEST_DIR 是一个环境变量，表示当前 Cargo 项目的根目录。include_dir! 宏会在编译时将指定路径下的文件和目录嵌入到二进制中，这样在运行时就可以直接访问这些文件，而不需要依赖外部文件系统。
static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../dist");

const USAGE: &str = "\
ccbuddy-server

用法:
  ccbuddy-server                  监听地址/端口取 ~/.ccbuddy/config.json（默认 127.0.0.1:18787）
  ccbuddy-server -p <端口>        本次运行覆盖端口（地址仍取配置）
  ccbuddy-server --set-password   设置/修改访问密码（交互输入，Argon2 哈希存储）
  ccbuddy-server --clear-password 清空当前密码（下次启动重新生成随机密码）
  ccbuddy-server -h              显示本帮助

配置文件 ~/.ccbuddy/config.json， 手动修改后重启生效。

安全:
  server 强制启用密码验证（应用内登录页 + session token），不存在无密码状态。
  首次启动自动生成随机密码并在终端打印一次，请登录后到设置页改成自己的。
  配置文件只存 Argon2 哈希，被他人读取也无法还原出明文。
  环境变量 CCBUDDY_PASSWORD 可临时覆盖（明文，仅进程内存）。
  注意: 本服务不自带 TLS，请勿直接暴露到公网；如需公网访问请置于 HTTPS 反代之后。
";

fn main() {
    // 参数解析：-p <端口> / --set-password / --clear-password / -h
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut port: Option<u16> = None;
    let mut set_password = false;
    let mut clear_password = false;

    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-p" | "--port" => {
                let raw = match it.next() {
                    Some(v) => v,
                    None => {
                        eprintln!("ccbuddy-server: 参数 -p 需要一个端口号（如 -p 9000）\n\n{USAGE}");
                        std::process::exit(2);
                    }
                };
                match raw.parse::<u16>() {
                    Ok(p) if p != 0 => port = Some(p),
                    _ => {
                        eprintln!("ccbuddy-server: 无效端口号: {raw}（需 1-65535）\n\n{USAGE}");
                        std::process::exit(2);
                    }
                }
            }
            "--set-password" => set_password = true,
            "--clear-password" => clear_password = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => {
                eprintln!("ccbuddy-server: 未知参数: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    if set_password || clear_password {
        // 管理操作与启动服务互斥，处理完直接退出
        let code = if set_password {
            if clear_password {
                eprintln!("ccbuddy-server: --set-password 与 --clear-password 不可同时使用\n\n{USAGE}");
                2
            } else {
                match set_pwd() {
                    Ok(msg) => {
                        println!("{msg}");
                        0
                    }
                    Err(e) => {
                        eprintln!("ccbuddy-server: {e}");
                        1
                    }
                }
            }
        } else {
            match clear_pwd() {
                Ok(msg) => {
                    println!("{msg}");
                    0
                }
                Err(e) => {
                    eprintln!("ccbuddy-server: {e}");
                    1
                }
            }
        };
        std::process::exit(code);
    }

    // 监听地址 = 配置文件（bind:port）；-p 只覆盖端口部分
    let mut addr = ccbuddy_lib::config_server_addr();
    if let Some(p) = port {
        let bind = addr.split(':').next().unwrap_or("127.0.0.1");
        addr = format!("{bind}:{p}");
    }
    ccbuddy_lib::run_server(&addr, &DIST);
}

/// 交互式设置访问密码：两次输入确认，Argon2 哈希后写入配置文件。
fn set_pwd() -> Result<String, String> {
    print!("请输入访问密码: ");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let first = rpassword::read_password().map_err(|e| format!("读取密码失败: {e}"))?;
    print!("请再次输入确认: ");
    let _ = std::io::stdout().flush();
    let second = rpassword::read_password().map_err(|e| format!("读取密码失败: {e}"))?;

    if first.is_empty() {
        return Err("密码不能为空（清空密码请用 --clear-password）".to_string());
    }
    if first != second {
        return Err("两次输入不一致".to_string());
    }

    let hashed = ccbuddy_lib::hash_password(&first)?;
    ccbuddy_lib::write_server_password(Some(&hashed))?;
    Ok(format!(
        "访问密码已设置（配置文件: {}）",
        ccbuddy_lib::config_path_display()
    ))
}

/// 清空访问密码（下次启动重新生成随机密码）。
fn clear_pwd() -> Result<String, String> {
    ccbuddy_lib::write_server_password(None)?;
    Ok(format!(
        "访问密码已清空（下次启动重新生成随机密码，配置文件: {}）",
        ccbuddy_lib::config_path_display()
    ))
}
