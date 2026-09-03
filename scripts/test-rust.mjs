#!/usr/bin/env bun

import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const [mode, ...inputArgs] = process.argv.slice(2);

function run(command, args, env = process.env) {
  const result = spawnSync(command, args, {
    cwd: root,
    env,
    stdio: 'inherit',
    shell: false,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

if (!['crate', 'core', 'desktop', 'browser'].includes(mode)) {
  console.error('usage: bun scripts/test-rust.mjs <crate|core|desktop|browser> [crate] [cargo test args]');
  process.exit(2);
}

run(process.execPath, ['scripts/prune-build.mjs']);

if (mode === 'crate') {
  const [packageName, ...cargoArgs] = inputArgs;
  if (!packageName || packageName.startsWith('-')) {
    console.error('usage: bun run test:crate <crate> [cargo test args]');
    process.exit(2);
  }
  run('cargo', ['test', '-p', packageName, ...cargoArgs]);
} else if (mode === 'browser') {
  // `browser-use` is not a default feature anywhere in the workspace, so the
  // focused lanes (crate/core) compile the feature-gated browser tests OUT and
  // silently skip them. This lane turns the feature on for every crate that
  // gates browser tests behind it, plus the ungated browser platform hub.
  // Integration-test-heavy crates are restricted to --lib: their gated
  // browser tests all live in the library, and this keeps the lane focused.
  run('cargo', ['test', '-p', 'nomifun-browser-platform', ...inputArgs]);
  run('cargo', ['test', '-p', 'nomifun-gateway', ...inputArgs]);
  run('cargo', ['test', '-p', 'nomifun-ai-agent', '--features', 'browser-use', '--lib', ...inputArgs]);
  run('cargo', ['test', '-p', 'nomi-agent', '--features', 'browser-use', '--lib', ...inputArgs]);
  run('cargo', ['test', '-p', 'nomifun-app', '--features', 'browser-use', '--lib', ...inputArgs]);
} else if (mode === 'core') {
  run('cargo', ['test', '--workspace', '--exclude', 'nomifun-desktop', ...inputArgs]);
} else {
  run(process.execPath, ['scripts/ensure-ui-dist.mjs']);
  const env = {
    ...process.env,
    TAURI_CONFIG: JSON.stringify({ bundle: { resources: [] } }),
  };
  run('cargo', ['test', '-p', 'nomifun-desktop', ...inputArgs], env);
}
