#!/usr/bin/env bun
/**
 * kill-stale-dev — kill leftover dev binaries running out of this repo's
 * `target/` directory.
 *
 * Why: agent sessions spawn CLI trees (e.g. `bunx → codex-acp → MCP stdio
 * bridges`), and the stdio bridges are the desktop binary itself
 * (`nomifun-desktop.exe mcp-*-stdio`). If the dev app dies without cleanup
 * (tauri dev rebuild, Ctrl+C, crash), the orphaned tree survives — and on
 * Windows a running image locks its exe, so the next `cargo build` fails
 * with `failed to remove file ... os error 5`. The in-process fix is the
 * Job Object/process-group ownership in `nomi-process-runtime`; this script is
 * the preflight safety net that also clears stale processes left by builds
 * that predate the supervised runtime, or by paths it cannot cover.
 *
 * Cross-platform; never fails the dev command (always exits 0).
 * Usage: bun scripts/kill-stale-dev.mjs [binary-name ...]   (default: nomifun-desktop)
 */
import { execSync } from 'node:child_process';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const isWin = process.platform === 'win32';
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
// Binary names come from package.json — keep them shell/WQL/regex-inert.
const names = process.argv.slice(2).filter((n) => /^[\w.-]+$/.test(n));
if (names.length === 0) names.push('nomifun-desktop');

/** Escape a literal string for use inside an extended regex (pgrep -f). */
const escapeRegex = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/** [{pid, path}] of live processes whose executable lives under `<repo>/target/`. */
function staleProcesses(name) {
  try {
    if (isWin) {
      const ps = `Get-CimInstance Win32_Process -Filter "Name='${name}.exe'" | Select-Object ProcessId,ExecutablePath | ConvertTo-Json -Compress`;
      const out = execSync(`powershell -NoProfile -Command "${ps.replace(/"/g, '\\"')}"`, {
        encoding: 'utf8',
      }).trim();
      if (!out) return [];
      const rows = JSON.parse(out);
      const list = Array.isArray(rows) ? rows : [rows];
      const prefix = `${repoRoot.toLowerCase()}\\target\\`;
      return list
        .filter((r) => (r.ExecutablePath || '').toLowerCase().startsWith(prefix))
        .map((r) => ({ pid: String(r.ProcessId), path: r.ExecutablePath }));
    }
    // macOS / Linux: pgrep -f matches the WHOLE command line, so anchor to
    // argv[0] — otherwise a debugger/editor/tail whose arguments merely
    // mention the path would be SIGKILLed too.
    // pgrep exits non-zero when nothing matches → caught below.
    const pattern = `^${escapeRegex(repoRoot)}/target/.*${escapeRegex(name)}`;
    const out = execSync(`pgrep -f "${pattern}"`, { encoding: 'utf8' });
    return out
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter((p) => p && p !== String(process.pid))
      .map((pid) => ({ pid, path: pattern }));
  } catch {
    return [];
  }
}

function kill(pid) {
  try {
    // /T also takes the process tree — bridges hang off third-party CLIs.
    if (isWin) execSync(`taskkill /PID ${pid} /T /F`, { stdio: 'ignore' });
    else execSync(`kill -9 ${pid}`, { stdio: 'ignore' });
    return true;
  } catch (e) {
    // taskkill 128 = "not found": an earlier /T tree-kill in this loop
    // already took this pid down with its ancestor. That's success.
    return isWin && e.status === 128;
  }
}

let killedAny = false;
for (const name of names) {
  const procs = staleProcesses(name);
  if (procs.length === 0) {
    console.log(`[kill-stale-dev] ${name}: no stale processes`);
    continue;
  }
  for (const { pid, path } of procs) {
    const ok = kill(pid);
    killedAny = true;
    console.log(`[kill-stale-dev] ${name}: ${ok ? 'killed' : 'FAILED to kill'} PID ${pid} (${path})`);
  }
}

// Give the OS a beat to release file locks before cargo tries to relink.
// Best-effort like everything here — this script must never break the chain.
if (killedAny && isWin) {
  try {
    execSync('powershell -NoProfile -Command "Start-Sleep -Milliseconds 400"', { stdio: 'ignore' });
  } catch {
    /* ignore */
  }
}
