#!/usr/bin/env python3
"""
ccbuddy 一键打包脚本（本地与 GitHub Actions 通用）。

产物输出到固定目录 `dist-release/`，文件名不带版本号
（GitHub `releases/latest/download/<文件名>` 地址固定，便于程序自动更新/下载）：

  dist-release/
    ccbuddy-hook-windows-x86_64.exe          # hook（各平台）
    ccbuddy-hook-linux-x86_64
    ccbuddy-hook-darwin-x86_64
    ccbuddy-hook-darwin-aarch64
    ccbuddy-windows-x86_64-setup.exe         # 主程序安装包
    ccbuddy-linux-x86_64.AppImage
    ccbuddy-darwin-aarch64.dmg
    ccbuddy-windows-x86_64-portable.zip      # 便携包（主程序 + hook，免安装）
    ccbuddy-linux-x86_64-portable.tar.gz
    ccbuddy-darwin-aarch64-portable.zip
    ccbuddy-server-linux-x86_64-musl         # 无头服务端（可选）

用法：
  python scripts/build.py            # 打包当前平台（主程序安装包 + 便携包 + hook）
  python scripts/build.py --hook-only  # 仅构建当前平台的 hook 二进制
  python scripts/build.py --server     # 额外构建当前平台的 ccbuddy-server
  python scripts/build.py --server-musl  # Linux musl 静态链接 server
  python scripts/build.py --all      # 尝试构建所有平台 hook（需本机有交叉工具链，CI 不用）

说明：
- hook 是纯 Rust 二进制（无 GUI 依赖），可交叉编译。
- Linux 的 hook 固定用 musl 静态链接（x86_64-unknown-linux-musl），
  不依赖 glibc，可在 Ubuntu 18 等旧版发行版上运行。
- Tauri 主程序依赖各平台原生 WebView，无法交叉编译，须在对应平台运行
  （GitHub Actions 用三平台 matrix，本地默认只打当前平台）。
- 便携包：裸主程序二进制 + 平台命名的 hook，解压即用；hook 与主程序同目录，
  程序内"一键安装"可直接识别（支持 ccbuddy-hook-<平台>-<架构> 命名）。
"""

import argparse
import platform
import shutil
import subprocess
import sys
from pathlib import Path

if sys.platform == "win32":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")


ROOT = Path(__file__).resolve().parent.parent  # ccbuddy/
SRC_TAURI = ROOT / "src-tauri"
OUT_DIR = ROOT / "dist-release"

# hook 交叉编译目标（triple -> 输出文件名后缀）
# Linux 固定用 musl 静态链接：hook 无 GUI 依赖，静态链接后不依赖 glibc，
# 可在任意旧版发行版（如 Ubuntu 18，glibc 2.27）直接运行。
MUSL_TARGET = "x86_64-unknown-linux-musl"

HOOK_TARGETS = [
    ("x86_64-pc-windows-msvc", "ccbuddy-hook-windows-x86_64.exe", None),
    (MUSL_TARGET, "ccbuddy-hook-linux-x86_64", None),
    ("x86_64-apple-darwin", "ccbuddy-hook-darwin-x86_64", "MACOSX_DEPLOYMENT_TARGET=10.13"),
    ("aarch64-apple-darwin", "ccbuddy-hook-darwin-aarch64", "MACOSX_DEPLOYMENT_TARGET=11.0"),
]


def run(cmd: list[str], cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    print(f"[build] {' '.join(cmd)}")
    # Windows 下 npm 是 npm.cmd，需要 shell 解析（命令为本脚本硬编码，无注入风险）
    subprocess.run(cmd, cwd=cwd, check=True, env=env, shell=sys.platform == "win32")


def ensure_hook_placeholder() -> None:
    """确保 resources 引用的占位文件存在（否则 tauri build.rs 检查失败）。"""
    bin_dir = SRC_TAURI / "binaries"
    bin_dir.mkdir(exist_ok=True)
    (bin_dir / "ccbuddy-hook").touch(exist_ok=True)
    if sys.platform == "win32":
        (bin_dir / "ccbuddy-hook.exe").touch(exist_ok=True)


def build_hook_current() -> Path:
    """构建当前平台的 hook（release），返回产物路径。

    Linux 固定用 musl 静态链接（x86_64-unknown-linux-musl）：hook 无 GUI 依赖，
    静态链接后不依赖 glibc，可在任意旧版发行版（如 Ubuntu 18）直接运行。
    """
    ensure_hook_placeholder()
    if sys.platform.startswith("linux"):
        run(["rustup", "target", "add", MUSL_TARGET])
        run(
            ["cargo", "build", "--release", "--bin", "ccbuddy-hook", "--target", MUSL_TARGET],
            cwd=SRC_TAURI,
        )
        return SRC_TAURI / "target" / MUSL_TARGET / "release" / "ccbuddy-hook"
    run(["cargo", "build", "--release", "--bin", "ccbuddy-hook"], cwd=SRC_TAURI)
    name = "ccbuddy-hook.exe" if sys.platform == "win32" else "ccbuddy-hook"
    return SRC_TAURI / "target" / "release" / name


def build_hook_cross(target: str, out_name: str, env_extra: str | None) -> bool:
    """尝试交叉编译 hook 到指定 target，成功返回 True。"""
    # 确认 target 已安装
    check = subprocess.run(
        ["rustup", "target", "list", "--installed"], capture_output=True, text=True
    )
    if target not in check.stdout:
        add = subprocess.run(["rustup", "target", "add", target], capture_output=True)
        if add.returncode != 0:
            print(f"[build] 跳过 {target}：无法安装 target")
            return False

    env = None
    if env_extra:
        key, _, val = env_extra.partition("=")
        env = {**__import__("os").environ, key: val}

    try:
        run(
            ["cargo", "build", "--release", "--bin", "ccbuddy-hook", "--target", target],
            cwd=SRC_TAURI,
            env=env,
        )
    except subprocess.CalledProcessError:
        print(f"[build] 跳过 {target}：交叉编译失败（可能缺少系统工具链）")
        return False

    src = SRC_TAURI / "target" / target / "release" / (
        "ccbuddy-hook.exe" if "windows" in target else "ccbuddy-hook"
    )
    shutil.copy2(src, OUT_DIR / out_name)
    print(f"[build] hook → {OUT_DIR / out_name}")
    return True


def build_server(target: str | None = None) -> Path | None:
    """
    构建无头服务端 ccbuddy-server（无桌面环境的 Linux 服务器使用）。

    前端 Vue 产物在编译时嵌入二进制（include_dir），构建前必须先 npm run build。
    target 传入时交叉编译（如 x86_64-unknown-linux-musl 静态链接产物，
    无任何系统依赖，可直接在任意 Linux 服务器运行）。
    """
    if not (ROOT / "dist" / "index.html").exists():
        print("[build] 前端产物不存在，先执行 npm run build ...")
        run(["npm", "run", "build"], cwd=ROOT)

    cmd = ["cargo", "build", "--release", "--no-default-features", "--bin", "ccbuddy-server"]
    if target:
        cmd += ["--target", target]
    ensure_hook_placeholder()
    run(cmd, cwd=SRC_TAURI)

    if target:
        bin_dir = SRC_TAURI / "target" / target / "release"
    else:
        bin_dir = SRC_TAURI / "target" / "release"
    name = "ccbuddy-server.exe" if sys.platform == "win32" else "ccbuddy-server"
    return bin_dir / name


def platform_id() -> tuple[str, str]:
    """当前平台标识 (plat, arch)，与 Rust 侧 hook_release_file_name 约定一致。"""
    plat = {"Windows": "windows", "Linux": "linux", "Darwin": "darwin"}[platform.system()]
    arch = "x86_64" if platform.machine().lower() in ("amd64", "x86_64") else "aarch64"
    return plat, arch


def build_app_current() -> list[Path]:
    """构建当前平台的 Tauri 主程序（安装包 + 便携包），返回产物路径列表。"""
    run(["npm", "run", "tauri", "build"], cwd=ROOT)

    plat, arch = platform_id()
    release_dir = SRC_TAURI / "target" / "release"
    bundle_dir = release_dir / "bundle"
    hook_name = "ccbuddy-hook.exe" if sys.platform == "win32" else "ccbuddy-hook"
    hook_release_name = f"ccbuddy-hook-{plat}-{arch}{hook_name.removeprefix('ccbuddy-hook')}"
    artifacts: list[Path] = []

    system = platform.system()
    if system == "Windows":
        # NSIS 安装包：bundle/nsis/CCBuddy_0.1.0_x64-setup.exe
        for p in (bundle_dir / "nsis").glob("*-setup.exe"):
            dest = OUT_DIR / f"ccbuddy-{plat}-{arch}-setup.exe"
            shutil.copy2(p, dest)
            artifacts.append(dest)
        # 便携包：裸主程序 + hook
        artifacts.append(
            make_portable_zip(
                OUT_DIR / f"ccbuddy-{plat}-{arch}-portable.zip",
                [(release_dir / "ccbuddy.exe", "ccbuddy.exe")],
                extra_files=[(OUT_DIR / hook_release_name, hook_release_name)],
            )
        )
    elif system == "Linux":
        # AppImage：bundle/appimage/ccbuddy_0.1.0_amd64.AppImage（本身即免安装）
        for p in (bundle_dir / "appimage").glob("*.AppImage"):
            dest = OUT_DIR / f"ccbuddy-{plat}-{arch}.AppImage"
            shutil.copy2(p, dest)
            artifacts.append(dest)
        artifacts.append(
            make_portable_targz(
                OUT_DIR / f"ccbuddy-{plat}-{arch}-portable.tar.gz",
                [(release_dir / "ccbuddy", "ccbuddy")],
                extra_files=[(OUT_DIR / hook_release_name, hook_release_name)],
            )
        )
    elif system == "Darwin":
        # dmg：bundle/dmg/CCBuddy_0.1.0_aarch64.dmg
        for p in (bundle_dir / "dmg").glob("*.dmg"):
            dest = OUT_DIR / f"ccbuddy-{plat}-{arch}.dmg"
            shutil.copy2(p, dest)
            artifacts.append(dest)
        # 便携包：.app 目录 + hook（ditto 保留符号链接与可执行权限）
        app_dirs = list((bundle_dir / "macos").glob("*.app"))
        if app_dirs:
            artifacts.append(
                make_portable_zip_darwin(
                    OUT_DIR / f"ccbuddy-{plat}-{arch}-portable.zip",
                    app_dirs[0],
                    extra_files=[(OUT_DIR / hook_release_name, hook_release_name)],
                )
            )

    for p in artifacts:
        print(f"[build] app → {p}")
    return artifacts


def make_portable_zip(dest: Path, files: list[tuple[Path, str]], extra_files: list[tuple[Path, str]] = []) -> Path:
    """Windows 便携包：zip 归档（stdlib，无外部依赖）。"""
    import zipfile

    with zipfile.ZipFile(dest, "w", zipfile.ZIP_DEFLATED) as zf:
        for src, name in files + extra_files:
            zf.write(src, name)
    return dest


def make_portable_targz(dest: Path, files: list[tuple[Path, str]], extra_files: list[tuple[Path, str]] = []) -> Path:
    """Linux 便携包：tar.gz 归档，保留可执行权限。"""
    import tarfile

    with tarfile.open(dest, "w:gz") as tf:
        for src, name in files + extra_files:
            ti = tf.gettarinfo(str(src), arcname=name)
            ti.mode = 0o755 if src.suffix not in (".json", ".txt") else 0o644
            with open(src, "rb") as f:
                tf.addfile(ti, f)
    return dest


def make_portable_zip_darwin(dest: Path, app_dir: Path, extra_files: list[tuple[Path, str]] = []) -> Path:
    """macOS 便携包：用 ditto 打 zip（保留符号链接/权限）。

    hook 复制进 .app/Contents/MacOS/（与主程序同目录，安装时可被识别），
    压缩后删除临时副本。
    """
    copies: list[Path] = []
    for src, name in extra_files:
        target = app_dir / "Contents" / "MacOS" / name
        shutil.copy2(src, target)
        copies.append(target)
    run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app_dir), str(dest)])
    for c in copies:
        c.unlink(missing_ok=True)
    return dest


def main() -> None:
    parser = argparse.ArgumentParser(description="ccbuddy 打包脚本")
    parser.add_argument("--hook-only", action="store_true", help="仅构建 hook 二进制")
    parser.add_argument("--server", action="store_true", help="额外构建无头服务端 ccbuddy-server")
    parser.add_argument("--server-musl", action="store_true",
                        help="构建 Linux musl 静态链接的 ccbuddy-server（仅 Linux 可用）")
    parser.add_argument("--all", action="store_true", help="尝试构建所有平台 hook（交叉编译）")
    args = parser.parse_args()

    OUT_DIR.mkdir(exist_ok=True)

    if args.all:
        # 交叉编译所有平台 hook（仅 hook，无主程序）
        for target, out_name, env in HOOK_TARGETS:
            build_hook_cross(target, out_name, env)
        return

    # 默认：当前平台主程序 + hook
    hook = build_hook_current()
    plat, arch = platform_id()
    hook_out = OUT_DIR / f"ccbuddy-hook-{plat}-{arch}{hook.suffix}"
    shutil.copy2(hook, hook_out)
    print(f"[build] hook → {hook_out}")

    if args.server:
        server = build_server()
        server_out = OUT_DIR / f"ccbuddy-server-{plat}-{arch}{server.suffix}"
        shutil.copy2(server, server_out)
        print(f"[build] server → {server_out}")

    if args.server_musl:
        if platform.system() != "Linux":
            print("[build] --server-musl 仅支持在 Linux 上构建")
            sys.exit(1)
        target = MUSL_TARGET
        run(["rustup", "target", "add", target])
        server = build_server(target)
        server_out = OUT_DIR / "ccbuddy-server-linux-x86_64-musl"
        shutil.copy2(server, server_out)
        print(f"[build] server(musl) → {server_out}")

    if not args.hook_only:
        build_app_current()

    print(f"[build] 完成，产物目录：{OUT_DIR}")


if __name__ == "__main__":
    main()
