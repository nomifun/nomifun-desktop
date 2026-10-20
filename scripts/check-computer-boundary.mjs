#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, isAbsolute, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const violations = [];
const retired = [
  /\bnomi_computer\b/, /\bnomi_a11y\b/, /\bwindows_computer\b/, /\bComputerTool\b/,
  /\bCOMPUTER_A11Y_[A-Z_]+\b/, /\bComputerA11y\b/,
  /nomifun\.computer-a11y/, /domain-computer-a11y/,
  /computer\/(?:a11y\.observe|observe|input|launch)(?=["'\s`]|$)/,
  /\bcomputer_(?:observe|inspect|input)\b/,
  /\/api\/computer\/permissions/,
];
const retiredPaths = ['crates/agent/nomi-computer/', 'crates/agent/nomi-a11y/',
  'crates/backend/nomifun-app/src/router/agent_role_host.rs',
  'crates/shared/nomi-system-execution/src/windows_computer.rs'];
const files = [...new Set(execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'], { cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 }).split('\0').filter(Boolean))];
for (const file of files) {
  const path = file.replaceAll('\\', '/');
  const absolute = resolve(root, path);
  if (!existsSync(absolute)) continue;
  if (retiredPaths.some(prefix => path.startsWith(prefix))) violations.push(`${path}: retired Computer implementation remains`);
  // Historical design records and the unmodified upstream snapshot are not
  // product consumers. Tests, fixtures and scripts remain in the source scan.
  if (path.startsWith('docs/') || path.startsWith('vendor/') || path === 'scripts/check-computer-boundary.mjs') continue;
  if (!/\.(?:rs|toml|json|[cm]?js|ts|tsx|sh|ps1)$/.test(path) || path.endsWith('Cargo.lock')) continue;
  const source = readFileSync(absolute, 'utf8');
  for (const pattern of retired) if (pattern.test(source)) violations.push(`${path}: retired Computer reference ${pattern}`);
  if (/computer/i.test(path) && /\bexpected_generation\b/.test(source)) violations.push(`${path}: retired Computer generation contract`);
}

const manifest = JSON.parse(execFileSync('cargo', ['metadata', '--format-version', '1', '--no-deps'], { cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 }));
const vendor = resolve(root, 'vendor/cua-driver');
function inVendor(path) {
  const delta = relative(vendor, resolve(path));
  return !isAbsolute(delta) && delta !== '..' && !delta.startsWith('../') && !delta.startsWith('..\\');
}
for (const pkg of manifest.packages) {
  if (inVendor(pkg.manifest_path)) violations.push(`${pkg.name}: driver joined the main workspace`);
  for (const dependency of pkg.dependencies) if (dependency.path && inVendor(dependency.path)) violations.push(`${pkg.name}: main workspace links vendored driver ${dependency.name}`);
}
const workerManifest = readFileSync(resolve(root, 'apps/computer-worker/Cargo.toml'), 'utf8');
if (!/^\[workspace\]/m.test(workerManifest)) violations.push('Computer worker must retain its independent workspace');
const noticeRoot = resolve(root, 'crates/backend/nomifun-runtime/notices');
const notices = JSON.parse(readFileSync(resolve(noticeRoot, 'manifest.json'), 'utf8'));
const driverNotices = notices.components.find(component => component.id === 'cua-driver');
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
if (!driverNotices?.files?.length) violations.push('Cua Driver and worker redistribution notices are missing');
else {
  const lock = sha256(readFileSync(resolve(root, 'apps/computer-worker/Cargo.lock')));
  if (driverNotices.worker_lock_sha256 !== lock) violations.push('Computer worker notices need regeneration for the current Cargo.lock');
  for (const file of driverNotices.files) {
    if (!/^[a-zA-Z0-9._-]+$/.test(file.filename) || !file.source.startsWith('https://')
      || sha256(readFileSync(resolve(noticeRoot, file.filename))) !== file.sha256) violations.push(`Computer notice provenance/digest drift: ${file.filename}`);
  }
}
const config = JSON.parse(readFileSync(resolve(root, 'apps/desktop/tauri.conf.json'), 'utf8'));
if (!config.bundle?.externalBin?.includes('binaries/nomifun-computer-worker')) violations.push('Tauri private worker externalBin is missing');
const hostManifest = readFileSync(resolve(root, 'crates/backend/nomifun-computer/Cargo.toml'), 'utf8');
if (/\b(?:cua-driver|platform-windows|platform-macos|platform-linux|enigo|x11rb|gstreamer)\b/.test(hostManifest)) violations.push('Computer host must use only the private stdio protocol, without native driver dependencies');

for (const args of [
  ['scripts/vendor/sync-cua-driver.mjs', '--check'],
  ['scripts/computer/generate-contracts.mjs', '--check'],
]) {
  try { execFileSync(process.execPath, args, { cwd: root, stdio: 'inherit' }); }
  catch { violations.push(`Computer source/generated check failed: ${args[0]}`); }
}
if (violations.length) {
  for (const violation of violations) console.error(violation);
  console.error(`Computer boundary failed: ${violations.length} violation(s)`);
  process.exitCode = 1;
} else console.log(`Computer boundary passed: ${files.length} tracked/untracked paths inspected; worker workspace isolated`);
