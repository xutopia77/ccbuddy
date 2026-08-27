//! ccbuddy-server：无头服务端入口（无桌面环境的 Linux 服务器）。
//!
//! 启动内嵌 HTTP 服务，托管与桌面端完全一致的 Vue 前端（编译产物嵌入二进制），
//! 浏览器访问即可查看 Claude Code 会话事件流。
//!
//! 用法：
//!   ccbuddy-server                  # 监听 127.0.0.1:8787
//!   ccbuddy-server 0.0.0.0:8787     # 指定监听地址
//!   CCBUDDY_ADDR=0.0.0.0:8787 ccbuddy-server
//!
//! 构建时要求前端产物 ../dist 已存在（打包脚本会先执行 npm run build）。

use include_dir::{include_dir, Dir};

static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../dist");

fn main() {
    let addr = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("CCBUDDY_ADDR").ok())
        .unwrap_or_else(|| "127.0.0.1:8787".to_string());
    ccbuddy_lib::run_server(&addr, &DIST);
}
