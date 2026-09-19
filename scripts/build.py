#!/usr/bin/env python3
"""
ccbuddy 一键打包脚本（本地与 GitHub Actions 通用）。

裸二进制一律叫 `ccbuddy-hook` / `ccbuddy-server`（不带平台、架构、musl 后缀）；
发布时统一打成压缩包区分平台，平台后缀只出现在压缩包名上。
产物输出到固定目录 `dist-release/`，文件名不带版本号
（GitHub Release 附件地址固定，便于用户直接下载）：

  dist-release/
    ccbuddy-hook-windows-x86_64.zip          # hook（包内 ccbuddy-hook.exe）
    ccbuddy-hook-linux-x86_64.tar.gz         # hook（包内 ccbuddy-hook，musl 静态）
    ccbuddy-hook-darwin-x86_64.tar.gz
    ccbuddy-hook-darwin-aarch64.tar.gz
    ccbuddy-server-linux-x86_64.tar.gz       # 无头服务端（包内 ccbuddy-server，musl 静态）
    ccbuddy-windows-x86_64-setup.exe         # 主程序安装包
    ccbuddy-linux-x86_64.AppImage
    ccbuddy-darwin-aarch64.dmg
    ccbuddy-windows-x86_64-portable.zip      # 便携包（主程序 + hook，免安装）
    ccbuddy-linux-x86_64-portable.tar.gz
    ccbuddy-darwin-aarch64-portable.zip

用法：
  python scripts/build.py            # Linux：hook + server（均 musl 静态）；Windows/macOS：主程序 + hook
  python scripts/build.py --hook-only  # 仅构建当前平台的 hook 二进制
  python scripts/build.py --server     # 额外构建当前平台的 ccbuddy-server（Linux 默认已含）
  python scripts/build.py --app     # 构建 Tauri GUI 主程序（Linux 默认跳过：
                                    #   依赖 WebKitGTK 系统库，装环境成本高，
                                    #   纯服务器部署场景无需 GUI）
  python scripts/build.py --hook-target aarch64-apple-darwin   # 交叉编译指定 target 的 hook
  python scripts/build.py --all      # 尝试构建所有平台 hook（需本机有交叉工具链，CI 不用）

说明：
- hook 是纯 Rust 二进制（无 GUI 依赖），可交叉编译。
- hook 产物在主程序编译前 copy 到 src-tauri/binaries/embedded-hook[.exe]，
  由 build.rs 嵌入主程序（server 与 GUI 共用），安装时解出，无运行期网络下载。
- Linux 的 hook 与 server 固定用 musl 静态链接（x86_64-unknown-linux-musl），
  不依赖 glibc，可直接在 Ubuntu 18 等旧版发行版 / 任意服务器上运行。
- Tauri 主程序依赖各平台原生 WebView，无法交叉编译，须在对应平台运行
  （GitHub Actions 用三平台 matrix，本地默认只打当前平台）。
- 便携包：裸主程序二进制 + 裸名 hook，解压即用；hook 与主程序同目录，
  程序内"一键安装"可直接识别（ccbuddy-hook 为标准候选名）。
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

# hook 交叉编译目标（triple, 额外环境变量）
# Linux 固定用 musl 静态链接：hook 无 GUI 依赖，静态链接后不依赖 glibc，
# 可在任意旧版发行版（如 Ubuntu 18，glibc 2.27）直接运行。
MUSL_TARGET = "x86_64-unknown-linux-musl"

HOOK_TARGETS = [
    ("x86_64-pc-windows-msvc", None),
    (MUSL_TARGET, None),
    ("x86_64-apple-darwin", "MACOSX_DEPLOYMENT_TARGET=10.13"),
    ("aarch64-apple-darwin", "MACOSX_DEPLOYMENT_TARGET=11.0"),
]


def triple_ident(target: str) -> tuple[str, str]:
    """Rust target triple → (plat, arch)，口径与 Rust 侧 hook_release_file_name 一致。"""
    arch = target.split("-")[0]
    if "darwin" in target or "apple" in target:
        return "darwin", arch
    if "windows" in target:
        return "windows", arch
    if "linux" in target:
        return "linux", arch
    return "unknown", arch


def archive_name(stem: str, plat: str, arch: str) -> str:
    """发布压缩包名：`<裸名>-<平台>-<架构>.<zip|tar.gz>`（压缩包是唯一带平台后缀的产物）。"""
    ext = ".zip" if plat == "windows" else ".tar.gz"
    return f"{stem}-{plat}-{arch}{ext}"


def make_release_archive(src: Path, stem: str, plat: str, arch: str) -> Path:
    """把裸二进制打成发布压缩包（包内文件名保持裸名 ccbuddy-hook / ccbuddy-server）。

    Windows 产 .zip（内含 .exe），其余平台产 .tar.gz（保留可执行位）。
    """
    dest = OUT_DIR / archive_name(stem, plat, arch)
    if dest.suffix == ".zip":
        return make_portable_zip(dest, [(src, src.name)])
    return make_portable_targz(dest, [(src, src.name)])


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

    必须加 --no-default-features：默认 gui feature 会把 tauri 及 Linux 桌面栈
    （dbus/tao 等）拉进依赖图，libdbus-sys 的 build.rs 在 musl 交叉编译时
    pkg-config 找不到目标 sysroot 而 panic。hook 是独立日志程序，不使用 lib，
    关掉 gui 不影响其功能（ccbuddy-server 的 musl 编译同样是 --no-default-features）。

    产物额外 copy 到 src-tauri/binaries/embedded-hook[.exe]：
    build.rs 会把它拷进 OUT_DIR，lib.rs 用 include_bytes! 嵌入主程序
    （server 与 GUI 共用），安装时解出到磁盘，替代运行期 GitHub 下载。
    本函数先于 build_server/build_app 执行，保证嵌入内容在主程序编译前就绪。
    """
    ensure_hook_placeholder()
    if sys.platform.startswith("linux"):
        run(["rustup", "target", "add", MUSL_TARGET])
        run(
            ["cargo", "build", "--release", "--no-default-features", "--bin", "ccbuddy-hook", "--target", MUSL_TARGET],
            cwd=SRC_TAURI,
        )
        hook = SRC_TAURI / "target" / MUSL_TARGET / "release" / "ccbuddy-hook"
    else:
        run(["cargo", "build", "--release", "--bin", "ccbuddy-hook"], cwd=SRC_TAURI)
        name = "ccbuddy-hook.exe" if sys.platform == "win32" else "ccbuddy-hook"
        hook = SRC_TAURI / "target" / "release" / name

    # 内嵌副本：文件名固定 embedded-hook（Windows 加 .exe），build.rs 按目标平台选取
    embedded_name = "embedded-hook.exe" if sys.platform == "win32" else "embedded-hook"
    embedded = SRC_TAURI / "binaries" / embedded_name
    embedded.parent.mkdir(exist_ok=True)
    shutil.copy2(hook, embedded)
    print(f"[build] hook 内嵌副本 → {embedded}")
    return hook


def build_hook_cross(target: str, env_extra: str | None) -> bool:
    """尝试交叉编译 hook 到指定 target 并打包，成功返回 True。"""
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

    # Linux musl 目标需关闭默认 gui feature（否则 libdbus-sys 交叉 pkg-config 失败）
    cmd = ["cargo", "build", "--release", "--bin", "ccbuddy-hook", "--target", target]
    if "linux-musl" in target:
        cmd.insert(2, "--no-default-features")

    try:
        run(cmd, cwd=SRC_TAURI, env=env)
    except subprocess.CalledProcessError:
        print(f"[build] 跳过 {target}：交叉编译失败（可能缺少系统工具链）")
        return False

    src = SRC_TAURI / "target" / target / "release" / (
        "ccbuddy-hook.exe" if "windows" in target else "ccbuddy-hook"
    )
    plat, arch = triple_ident(target)
    dest = make_release_archive(src, "ccbuddy-hook", plat, arch)
    print(f"[build] hook → {dest}（内含 {src.name}）")
    return True


def built_hook_path() -> Path:
    """当前平台已构建的 hook 裸二进制路径（build_hook_current 的产物，便携包内嵌用）。"""
    name = "ccbuddy-hook.exe" if sys.platform == "win32" else "ccbuddy-hook"
    if sys.platform.startswith("linux"):
        return SRC_TAURI / "target" / MUSL_TARGET / "release" / name
    return SRC_TAURI / "target" / "release" / name


def build_server(target: str | None = None) -> Path:
    """
    构建无头服务端 ccbuddy-server（无桌面环境的 Linux 服务器使用）。

    前端 Vue 产物在编译时嵌入二进制（include_dir），构建前必须先 npm run build。
    target 传入时交叉编译（Linux 默认传 musl target：静态链接产物无任何系统依赖，
    可直接在任意 Linux 服务器运行）。
    """
    if not (ROOT / "dist" / "index.html").exists():
        print("[build] 前端产物不存在，先执行 npm run build ...")
        run(["npm", "run", "build"], cwd=ROOT)

    cmd = ["cargo", "build", "--release", "--no-default-features", "--bin", "ccbuddy-server"]
    if target:
        run(["rustup", "target", "add", target])
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
    # 便携包附带裸名 hook（ccbuddy-hook[.exe] 是安装逻辑的标准候选名）
    hook_bin = built_hook_path()
    hook_name = hook_bin.name
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
                extra_files=[(hook_bin, hook_name)],
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
                extra_files=[(hook_bin, hook_name)],
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
                    extra_files=[(hook_bin, hook_name)],
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
    parser.add_argument("--server", action="store_true", help="额外构建无头服务端 ccbuddy-server（Linux 默认构建）")
    parser.add_argument("--app", action="store_true",
                        help="构建 Tauri GUI 主程序（Linux 默认跳过：依赖 WebKitGTK 系统库，"
                             "仅服务器部署场景无需）")
    parser.add_argument("--hook-target", metavar="TRIPLE",
                        help="交叉编译指定 rust target 的 hook 并打包（如 aarch64-apple-darwin）")
    parser.add_argument("--all", action="store_true", help="尝试构建所有平台 hook（交叉编译）")
    args = parser.parse_args()

    OUT_DIR.mkdir(exist_ok=True)

    if args.all:
        # 交叉编译所有平台 hook（仅 hook，无主程序）
        for target, env in HOOK_TARGETS:
            build_hook_cross(target, env)
        return

    if args.hook_target:
        # 单一 target 交叉编译（macOS 发布用：在一种架构上补齐另一种架构的 hook）
        if not build_hook_cross(args.hook_target, None):
            sys.exit(1)
        return

    # Linux：server 依赖轻（纯 Rust 无 GUI）且用 musl 静态链接，默认构建；
    # GUI 主程序依赖 WebKitGTK 系统库，仅 --app 显式请求时构建。
    # Windows/macOS：主程序默认构建，--server 可选。
    on_linux = platform.system() == "Linux"
    want_server = args.server or (on_linux and not args.hook_only)
    want_app = args.app or (not on_linux and not args.hook_only)

    # 当前平台 hook
    hook = build_hook_current()
    plat, arch = platform_id()
    hook_out = make_release_archive(hook, "ccbuddy-hook", plat, arch)
    print(f"[build] hook → {hook_out}（内含 {hook.name}）")

    if want_server:
        # Linux 默认 musl 静态链接：产物无系统依赖，可在任意发行版直接运行
        server = build_server(MUSL_TARGET if on_linux else None)
        sp, sa = triple_ident(MUSL_TARGET) if on_linux else (plat, arch)
        server_out = make_release_archive(server, "ccbuddy-server", sp, sa)
        print(f"[build] server → {server_out}（内含 {server.name}）")

    if want_app:
        build_app_current()

    print(f"[build] 完成，产物目录：{OUT_DIR}")


if __name__ == "__main__":
    main()
