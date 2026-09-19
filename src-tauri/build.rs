use std::path::PathBuf;

fn main() {
    // 仅在启用 gui feature（桌面版）时运行 tauri 构建；
    // ccbuddy-server（--no-default-features）不依赖 tauri，跳过以支持纯服务器交叉编译。
    // build.rs 无法直接用 #[cfg(feature)]（它是独立编译的 crate），改用 Cargo 注入的环境变量判断。
    if std::env::var_os("CARGO_FEATURE_GUI").is_some() {
        tauri_build::build()
    }

    // 内嵌 hook 来源：build.py 先编 hook 再编主程序，产物放在
    // src-tauri/binaries/embedded-hook[.exe]（server 与 GUI 共用一份）。
    // 不存在时写 0 字节占位（裸 cargo build / 未走 build.py 流程），
    // 不阻断编译——运行期靠空判定走"手动放置"提示。
    //
    // 注意：build.rs 里不能用相对路径（工作目录不保证），
    // 必须用 CARGO_MANIFEST_DIR / OUT_DIR 环境变量拼绝对路径。
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("缺少 CARGO_MANIFEST_DIR 环境变量"),
    );
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("缺少 OUT_DIR 环境变量"));

    // 目标平台是 Windows 时产物带 .exe 后缀（交叉编译时看 target 而非 host）
    let is_target_windows =
        std::env::var_os("CARGO_CFG_TARGET_OS").as_deref() == Some(std::ffi::OsStr::new("windows"));
    let src_name = if is_target_windows { "embedded-hook.exe" } else { "embedded-hook" };
    let src = manifest.join("binaries").join(src_name);
    let dst = out_dir.join("embedded-hook.bin");

    // 目录整体纳入重编译判定：占位文件出现/更新都会触发本脚本重跑，
    // 从而让 include_bytes! 拿到新内容
    println!("cargo:rerun-if-changed=binaries");

    if src.is_file() {
        std::fs::copy(&src, &dst).expect("复制内嵌 hook 到 OUT_DIR 失败");
    } else {
        // 0 字节占位：保证 include_bytes! 的目标文件始终存在
        std::fs::write(&dst, []).expect("写入内嵌 hook 占位文件失败");
    }
}
