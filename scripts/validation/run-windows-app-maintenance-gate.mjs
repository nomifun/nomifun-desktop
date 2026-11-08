#!/usr/bin/env node
// Release admission: the built application's independent helper must pass real
// Windows process/file/IPC tests and the production NSIS template before upload.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { runAppMaintenanceNativeSmoke } from './app-maintenance-native-smoke.mjs';
import { runAppMaintenancePackageSmoke } from './app-maintenance-package-smoke.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

function buildFixture() {
  const result = spawnSync('cargo', ['build', '--locked', '-p', 'nomifun-desktop-maintenance',
    '--example', 'maintenance_fixture', '--features', 'maintenance-validation', '--message-format=json-render-diagnostics'],
  { cwd: root, encoding: 'utf8', windowsHide: true, maxBuffer: 8 * 1024 * 1024 });
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error) throw result.error;
  assert.equal(result.status, 0, 'Failed to build the isolated maintenance fixture');
  let binary;
  for (const line of result.stdout.split(/\r?\n/)) {
    let event;
    try { event = JSON.parse(line); } catch { continue; }
    if (event.message?.rendered) process.stderr.write(event.message.rendered);
    if (event.reason === 'compiler-artifact' && event.target?.name === 'maintenance_fixture' && event.executable) binary = resolve(root, event.executable);
  }
  assert(binary && existsSync(binary), 'Cargo did not report the maintenance fixture executable');
  return binary;
}

export async function runWindowsAppMaintenanceGate({ helper, reportDirectory } = {}) {
  assert.equal(process.platform, 'win32', 'Windows release admission requires a native Windows host');
  assert(helper && isAbsolute(helper) && existsSync(helper), 'Pass the absolute built --helper executable');
  const directory = reportDirectory ?? mkdtempSync(join(tmpdir(), 'nomifun-app-maintenance-release-'));
  mkdirSync(directory, { recursive: true });
  const fixture = buildFixture();
  const native = await runAppMaintenanceNativeSmoke({ helper, fixture, report: join(directory, 'native.json') });
  if (!native.ok) return { ok: false, directory, native };
  const packaging = await runAppMaintenancePackageSmoke({ helper, fixture, report: join(directory, 'package.json') });
  return { ok: packaging.ok, directory, native, packaging };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const options = {};
    for (let index = 2; index < process.argv.length; index += 2) {
      const name = process.argv[index];
      assert(['--helper', '--report-directory'].includes(name), `Unknown argument: ${name}`);
      assert(process.argv[index + 1], `Missing value: ${name}`);
      options[name === '--helper' ? 'helper' : 'reportDirectory'] = resolve(process.argv[index + 1]);
    }
    const result = await runWindowsAppMaintenanceGate(options);
    console.log(JSON.stringify(result));
    process.exitCode = result.ok ? 0 : 1;
  } catch (error) {
    console.error(error.stack ?? String(error));
    process.exitCode = 1;
  }
}
