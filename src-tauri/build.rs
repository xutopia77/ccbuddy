fn main() {
    // 仅在启用 gui feature（桌面版）时运行 tauri 构建；
    // ccbuddy-server（--no-default-features）不依赖 tauri，跳过以支持纯服务器交叉编译。
    // build.rs 无法直接用 #[cfg(feature)]（它是独立编译的 crate），改用 Cargo 注入的环境变量判断。
    if std::env::var_os("CARGO_FEATURE_GUI").is_some() {
        tauri_build::build()
    }
}
