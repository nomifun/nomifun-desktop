#!/usr/bin/env bun
/**
 * free-ports — kill whatever process is LISTENING on the given TCP port(s).
 *
 * Used as a preflight for the dev commands (`dev`, `dev:web`, `serve:web`).
 * On Windows especially, Ctrl-C'ing `tauri dev` / `concurrently` often leaves the
 * spawned Vite (`node vite.js`) or backend child orphaned, still holding 5173 /
 * 8787. Because Vite is pinned with `strictPort: true` (it MUST match Tauri's
 * fixed `devUrl` :5173) and `dev:web` binds both processes with `concurrently -k`,
 * a single stale listener makes the whole command fail with "Port already in use".
 * Clearing the port first makes the next start self-healing.
 *
 * Cross-platform: Windows (netstat + taskkill), macOS/Linux (lsof + kill).
 * Usage: bun scripts/free-ports.mjs 5173 8787
 */
import { execSync } from 'node:child_process';

const isWin = process.platform === 'win32';
const ports = process.argv.slice(2).map((p) => p.trim()).filter(Boolean);

if (ports.length === 0) {
  console.log('[free-ports] no ports given, nothing to do');
  process.exit(0);
}

/** PIDs (as strings) of processes LISTENING on `port`, excluding this script. */
function pidsOnPort(port) {
  const self = String(process.pid);
  try {
    if (isWin) {
      // Lines look like: "  TCP    127.0.0.1:5173   0.0.0.0:0   LISTENING   13880"
      // (UDP rows have no LISTENING state, so the filter naturally excludes them.)
      const out = execSync('netstat -ano', { encoding: 'utf8' });
      const pids = new Set();
      for (const line of out.split(/\r?\n/)) {
        if (!line.includes('LISTENING')) continue;
        const cols = line.trim().split(/\s+/);
        const local = cols[1] || '';
        const pid = cols[cols.length - 1];
        // `:5173` (with the colon) anchors the match so :35173 won't false-hit.
        if (local.endsWith(`:${port}`) && /^\d+$/.test(pid)) pids.add(pid);
      }
      return [...pids].filter((p) => p !== self);
    }
    // macOS / Linux. lsof exits non-zero when nothing matches → caught below.
    const out = execSync(`lsof -nP -iTCP:${port} -sTCP:LISTEN -t`, { encoding: 'utf8' });
    return out
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter((p) => p && p !== self);
  } catch {
    return [];
  }
}

function kill(pid) {
  try {
    // /T also takes the process tree, matching the orphaned-child case.
    if (isWin) execSync(`taskkill /PID ${pid} /T /F`, { stdio: 'ignore' });
    else execSync(`kill -9 ${pid}`, { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
}

let killedAny = false;
for (const port of ports) {
  const pids = pidsOnPort(port);
  if (pids.length === 0) {
    console.log(`[free-ports] ${port}: already free`);
    continue;
  }
  for (const pid of pids) {
    const ok = kill(pid);
    killedAny = true;
    console.log(`[free-ports] ${port}: ${ok ? 'freed (killed' : 'FAILED to kill'} PID ${pid}${ok ? ')' : ''}`);
  }
}

// Give the OS a beat to release the socket before the dev server tries to bind.
if (killedAny && isWin) execSync('powershell -NoProfile -Command "Start-Sleep -Milliseconds 400"', { stdio: 'ignore' });
