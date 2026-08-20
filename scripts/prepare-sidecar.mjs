// 在 `tauri build` 打包前调用：构建 ccbuddy-hook 并复制到 resources 目录，
// 使 bundle.resources 能将其打进安装包，运行时从 resource_dir 读取
// （install_hooks 从 resource_dir 复制 ccbuddy-hook.exe 到 ~/.claude/）。
//
// 注意：tauri 的 build.rs 会在 cargo build 时检查 resources 引用的文件是否存在，
// 而 ccbuddy-hook 又是 cargo 自身的 [[bin]]（存在“先有鸡还是先有蛋”问题）。
// 因此先写一个占位文件让 build.rs 检查通过，构建出真实 hook 后再覆盖它。
import { execSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const root = process.cwd(); // ccbuddy/
const srcTauri = join(root, 'src-tauri');
const hookName = process.platform === 'win32' ? 'ccbuddy-hook.exe' : 'ccbuddy-hook';

const binDir = join(srcTauri, 'binaries');
mkdirSync(binDir, { recursive: true });
const dest = join(binDir, hookName);

// 1. 占位：确保 resources 引用的文件存在（否则 build.rs 检查失败）
if (!existsSync(dest)) {
  writeFileSync(dest, '');
  console.log(`[sidecar] 已创建占位文件 → ${dest}`);
}

// 2. 构建真正的 hook（此时 build.rs 能看到占位文件，检查通过）
console.log('[sidecar] 构建 ccbuddy-hook ...');
execSync('cargo build --release --bin ccbuddy-hook', {
  cwd: srcTauri,
  stdio: 'inherit',
});

// 3. 用真实 hook 覆盖占位文件
const hook = join(srcTauri, 'target', 'release', hookName);
copyFileSync(hook, dest);
console.log(`[sidecar] 已替换为真实 hook → ${dest}`);
