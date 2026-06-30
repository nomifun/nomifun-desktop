#!/usr/bin/env bun
/**
 * run-win-build -- launcher for the Windows packaging script.
 *
 * `desktop-build-win.ps1` is PowerShell. We invoke it through whichever
 * PowerShell is installed, preferring PowerShell 7+ (`pwsh`) for its correct
 * UTF-8 console handling, and falling back to the always-present Windows
 * PowerShell 5.1 (`powershell.exe`). The previous `pwsh -File ...` entry in
 * package.json broke on machines that only have 5.1 installed.
 *
 * `-ExecutionPolicy Bypass` is required for the 5.1 fallback: its default policy
 * (Restricted/RemoteSigned) would otherwise refuse to run the local .ps1.
 *
 * All argv (architecture selection, `--signed`, `-- <tauri passthrough>`) is
 * forwarded verbatim to the script.
 */
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// Windows-only: the script bundles MSI/NSIS. macOS/Linux use build:mac/build:linux.
if (process.platform !== 'win32') {
  console.error('build:win 只能在 Windows 上运行。macOS 包用 build:mac，Linux 包用 build:linux。');
  process.exit(1);
}

/** First PowerShell that launches successfully: pwsh (7+) preferred, else powershell (5.1). */
function resolveShell() {
  for (const exe of ['pwsh', 'powershell']) {
    const probe = spawnSync(exe, ['-NoProfile', '-Command', '$PSVersionTable.PSVersion.Major'], { stdio: 'ignore' });
    if (!probe.error && probe.status === 0) return exe;
  }
  return null;
}

const shell = resolveShell();
if (!shell) {
  console.error('未找到 PowerShell（pwsh 或 powershell.exe）。请确认 Windows PowerShell 可用后重试。');
  process.exit(1);
}

const scriptPath = fileURLToPath(new URL('./desktop-build-win.ps1', import.meta.url));
const psArgs = ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', scriptPath, ...process.argv.slice(2)];

const result = spawnSync(shell, psArgs, { stdio: 'inherit' });
if (result.error) {
  console.error(`启动 ${shell} 失败:`, result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
