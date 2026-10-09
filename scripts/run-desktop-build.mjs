#!/usr/bin/env bun
// Every macOS product build signs and verifies its final app before distribution.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export function desktopBuildCommand(platform, args) {
  return platform === 'darwin'
    ? { command: 'bash', args: [fileURLToPath(new URL('./desktop-build-mac.sh', import.meta.url)), ...args], prune: false }
    : { command: 'bun', args: ['x', 'tauri', 'build', '--config', 'apps/desktop/tauri.conf.json', ...args], prune: true };
}

if (import.meta.main) {
  const forwarded = process.argv.slice(2).filter(argument => argument !== '--');
  const invocation = desktopBuildCommand(process.platform, forwarded);
  const built = spawnSync(invocation.command, invocation.args, { stdio: 'inherit', env: { ...process.env, CI: 'true' } });
  if (built.error) throw built.error;
  if (built.status !== 0) process.exit(built.status ?? 1);
  if (invocation.prune) {
    const pruned = spawnSync('bun', ['scripts/prune-build.mjs', '--post'], { stdio: 'inherit' });
    if (pruned.error) throw pruned.error;
    process.exit(pruned.status ?? 1);
  }
}
