#!/usr/bin/env python3
"""
ccbuddy 一键打包脚本（本地与 GitHub Actions 通用）。

产物输出到固定目录 `dist-release/`，便于 GitHub Actions 上传 Release：

  dist-release/
    ccbuddy-hook-windows-x86_64.exe
    ccbuddy-hook-linux-x86_64
    ccbuddy-hook-darwin-x86_64
    ccbuddy-hook-darwin-aarch64
    ccbuddy-<version>-windows-x86_64.exe      # 主程序（Tauri 安装包）
    ccbuddy-<version>-linux-x86_64.AppImage    # 主程序（Tauri 安装包）
    ccbuddy-<version>-darwin-aarch64.dmg       # 主程序（Tauri 安装包）

用法：
  python scripts/build.py            # 打包当前平台（主程序 + hook）
  python scripts/build.py --hook-only  # 仅构建当前平台的 hook 二进制
  python scripts/build.py --all      # 尝试构建所有平台 hook（需本机有交叉工具链，CI 不用）

说明：
- hook 是纯 Rust 二进制（无 GUI 依赖），可交叉编译。
- Tauri 主程序依赖各平台原生 WebView，无法交叉编译，须在对应平台运行
  （GitHub Actions 用三平台 matrix，本地默认只打当前平台）。
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
HOOK_TARGETS = [
    ("x86_64-pc-windows-msvc", "ccbuddy-hook-windows-x86_64.exe", None),
    ("x86_64-unknown-linux-gnu", "ccbuddy-hook-linux-x86_64", None),
    ("x86_64-apple-darwin", "ccbuddy-hook-darwin-x86_64", "MACOSX_DEPLOYMENT_TARGET=10.13"),
    ("aarch64-apple-darwin", "ccbuddy-hook-darwin-aarch64", "MACOSX_DEPLOYMENT_TARGET=11.0"),
]


def run(cmd: list[str], cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    print(f"[build] {' '.join(cmd)}")
    # Windows 下 npm 是 npm.cmd，需要 shell 解析（命令为本脚本硬编码，无注入风险）
    subprocess.run(cmd, cwd=cwd, check=True, env=env, shell=sys.platform == "win32")


def read_version() -> str:
    """从 tauri.conf.json 读取版本号。"""
    import json

    conf = json.loads((SRC_TAURI / "tauri.conf.json").read_text(encoding="utf-8"))
    return conf["version"]


def ensure_hook_placeholder() -> None:
    """确保 resources 引用的占位文件存在（否则 tauri build.rs 检查失败）。"""
    bin_dir = SRC_TAURI / "binaries"
    bin_dir.mkdir(exist_ok=True)
    (bin_dir / "ccbuddy-hook").touch(exist_ok=True)
    if sys.platform == "win32":
        (bin_dir / "ccbuddy-hook.exe").touch(exist_ok=True)


def build_hook_current() -> Path:
    """构建当前平台的 hook（release），返回产物路径。"""
    ensure_hook_placeholder()
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


def build_app_current() -> list[Path]:
    """构建当前平台的 Tauri 主程序（含前端与 hook sidecar），返回安装包产物列表。"""
    run(["npm", "run", "tauri", "build"], cwd=ROOT)

    # Tauri bundle 产物目录：src-tauri/target/release/bundle/
    bundle_dir = SRC_TAURI / "target" / "release" / "bundle"
    version = read_version()
    artifacts: list[Path] = []

    system = platform.system()
    if system == "Windows":
        # NSIS 安装包：bundle/nsis/CCBuddy_0.1.0_x64-setup.exe
        for p in (bundle_dir / "nsis").glob("*-setup.exe"):
            artifacts.append(p)
    elif system == "Linux":
        # AppImage：bundle/appimage/ccbuddy_0.1.0_amd64.AppImage
        for p in (bundle_dir / "appimage").glob("*.AppImage"):
            artifacts.append(p)
    elif system == "Darwin":
        # dmg：bundle/dmg/CCBuddy_0.1.0_aarch64.dmg
        for p in (bundle_dir / "dmg").glob("*.dmg"):
            artifacts.append(p)

    # 统一重命名：ccbuddy-<version>-<platform>-<arch>.<ext>
    renamed: list[Path] = []
    plat = {"Windows": "windows", "Linux": "linux", "Darwin": "darwin"}[system]
    arch = "x86_64" if platform.machine().lower() in ("amd64", "x86_64") else "aarch64"
    for p in artifacts:
        ext = p.suffix  # .exe / .AppImage / .dmg
        dest = OUT_DIR / f"ccbuddy-{version}-{plat}-{arch}{ext}"
        shutil.copy2(p, dest)
        renamed.append(dest)
        print(f"[build] app → {dest}")
    return renamed


def main() -> None:
    parser = argparse.ArgumentParser(description="ccbuddy 打包脚本")
    parser.add_argument("--hook-only", action="store_true", help="仅构建 hook 二进制")
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
    version = read_version()
    plat = {"Windows": "windows", "Linux": "linux", "Darwin": "darwin", "Java": "unknown"}[
        platform.system()
    ]
    arch = "x86_64" if platform.machine().lower() in ("amd64", "x86_64") else "aarch64"
    hook_out = OUT_DIR / f"ccbuddy-hook-{plat}-{arch}{hook.suffix}"
    shutil.copy2(hook, hook_out)
    print(f"[build] hook → {hook_out}")

    if not args.hook_only:
        build_app_current()

    print(f"[build] 完成，产物目录：{OUT_DIR}")


if __name__ == "__main__":
    main()
