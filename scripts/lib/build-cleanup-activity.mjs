import { spawnSync } from 'node:child_process';

// Process names only. Conservatively retain caches when another checkout is
// building too; neither command lines nor environment values are needed here.
const BUILD_PROCESS_NAMES = new Set([
  'cargo', 'rustc', 'rustdoc',
  'rust-lld', 'lld', 'lld-link', 'ld', 'ld.bfd', 'ld.gold', 'ld.lld',
  'ld64', 'ld64.lld', 'wasm-ld', 'link', 'collect2', 'mold',
  'cc', 'c++', 'gcc', 'g++', 'clang', 'clang++', 'clang-cl',
]);

function processName(value) {
  return value.trim().split(/[\\/]/).pop().toLowerCase().replace(/\.exe$/, '');
}

/**
 * A best-effort process snapshot, not a shared Cargo lock. A build can start
 * after this check; callers must not claim mutual exclusion with compilation.
 * Failed/unsupported enumeration is unknown and must prevent cleanup.
 */
export function inspectBuildActivity({ platform = process.platform, run = spawnSync } = {}) {
  if (!['win32', 'darwin', 'linux'].includes(platform)) return { state: 'unknown' };
  const windows = platform === 'win32';
  const command = windows ? 'powershell.exe' : 'ps';
  const args = windows
    ? [
        '-NoLogo', '-NoProfile', '-NonInteractive', '-WindowStyle', 'Hidden',
        '-Command',
        'try { @(Get-Process -ErrorAction Stop | Select-Object -ExpandProperty ProcessName) | ConvertTo-Json -Compress } catch { exit 1 }',
      ]
    : ['-A', '-o', 'comm='];
  try {
    const result = run(command, args, {
      encoding: 'utf8', windowsHide: true, shell: false,
      timeout: 10_000, maxBuffer: 4 * 1024 * 1024,
    });
    if (result.error || result.status !== 0 || typeof result.stdout !== 'string') {
      return { state: 'unknown' };
    }
    let names;
    if (windows) {
      const parsed = JSON.parse(result.stdout.replace(/^\uFEFF/, '').trim());
      names = Array.isArray(parsed) ? parsed : [parsed];
      if (names.length === 0 || names.some(name => typeof name !== 'string' || !name.trim())) {
        return { state: 'unknown' };
      }
    } else {
      names = result.stdout.split(/\r?\n/).map(name => name.trim()).filter(Boolean);
      if (names.length === 0) return { state: 'unknown' };
    }
    const active = [...new Set(names.map(processName).filter(name => BUILD_PROCESS_NAMES.has(name)))].sort();
    return active.length ? { state: 'busy', names: active } : { state: 'idle' };
  } catch {
    return { state: 'unknown' };
  }
}
