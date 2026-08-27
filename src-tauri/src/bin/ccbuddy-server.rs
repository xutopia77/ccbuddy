//! ccbuddy-server：无头服务端入口（无桌面环境的 Linux 服务器）。
//!
//! 启动内嵌 HTTP 服务，浏览器访问 Web 界面查看 Claude Code 会话事件流。
//!
//! 用法：
//!   ccbuddy-server                  # 监听 127.0.0.1:8787
//!   ccbuddy-server 0.0.0.0:8787     # 指定监听地址
//!   CCBUDDY_ADDR=0.0.0.0:8787 ccbuddy-server

fn main() {
    let addr = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("CCBUDDY_ADDR").ok())
        .unwrap_or_else(|| "127.0.0.1:8787".to_string());
    ccbuddy_lib::run_server(&addr);
}
