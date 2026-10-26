#!/usr/bin/env bun
// Product builds use the platform's complete application packaging pipeline.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { buildComputerWorker, rustHostTarget } from './computer/build-worker.mjs';
import { buildKeyboardMaintenance } from './build-keyboard-maintenance.mjs';

export function desktopBuildCommand(platform, args) {
  if (platform === 'darwin' || platform === 'linux') {
    const script = platform === 'darwin' ? 'desktop-build-mac.sh' : 'desktop-build-linux.sh';
    return { command: 'bash', args: [fileURLToPath(new URL(`./${script}`, import.meta.url)), ...args], prune: false };
  }
  return { command: 'bun', args: ['x', 'tauri', 'build', '--config', 'apps/desktop/tauri.conf.json', ...args], prune: true };
}

if (import.meta.main) {
  const forwarded = process.argv.slice(2).filter(argument => argument !== '--');
  if (process.platform === 'win32') {
    const targetIndex = forwarded.findIndex(argument => argument === '--target' || argument === '-t');
    const target = forwarded.find(argument => argument.startsWith('--target='))?.slice('--target='.length)
      ?? (targetIndex >= 0 ? forwarded[targetIndex + 1] : rustHostTarget());
    await buildComputerWorker({ target, profile: forwarded.includes('--debug') || forwarded.includes('-d') ? 'debug' : 'release' });
    // Keep Cargo's implicit/explicit target selection identical to Tauri's so
    // the helper is the actual sibling of MAINBINARYSRCPATH, including custom
    // CARGO_TARGET_DIR and debug builds.
    const explicitTarget = forwarded.some(argument => argument.startsWith('--target=')) || targetIndex >= 0;
    await buildKeyboardMaintenance({ target: explicitTarget ? target : undefined,
      profile: forwarded.includes('--debug') || forwarded.includes('-d') ? 'debug' : 'release' });
  }
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
