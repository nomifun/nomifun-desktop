#!/usr/bin/env bun
// Product builds use the platform's complete application packaging pipeline.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

export function desktopBuildCommand(platform, args) {
  if (platform === 'darwin' || platform === 'linux') {
    const script = platform === 'darwin' ? 'desktop-build-mac.sh' : 'desktop-build-linux.sh';
    return { command: 'bash', args: [fileURLToPath(new URL(`./${script}`, import.meta.url)), ...args], prune: false };
  }
  return { command: 'bun', args: ['x', 'tauri', 'build', '--config', 'apps/desktop/tauri.conf.json', ...args], prune: true };
}

if (import.meta.main) {
  const forwarded = process.argv.slice(2).filter(argument => argument !== '--');
  const invocation = desktopBuildCommand(process.platform, forwarded);
  const built = spawnSync(invocation.command, invocation.args, { stdio: 'inherit', env: { ...process.env, CI: 'true', NOMIFUN_EMBED_OFFICECLI: '1' } });
  if (built.error) throw built.error;
  if (built.status !== 0) process.exit(built.status ?? 1);
  if (invocation.prune) {
    const pruned = spawnSync('bun', ['scripts/prune-build.mjs', '--post'], { stdio: 'inherit' });
    if (pruned.error) throw pruned.error;
    process.exit(pruned.status ?? 1);
  }
}
