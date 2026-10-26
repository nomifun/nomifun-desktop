#!/usr/bin/env node
// Build the small, resource-independent helper next to the desktop binary.
// NSIS embeds this known package version in both installer and uninstaller.
import { spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { resolve } from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));

export function maintenanceBuildArgs({ target, profile = 'release' } = {}) {
  if (target && !/^[a-z0-9_]+-(?:[a-z0-9_]+-)+[a-z0-9_]+$/.test(target)) throw new Error('A concrete Rust target triple is required');
  if (!['debug', 'release'].includes(profile)) throw new Error(`Unsupported maintenance build profile: ${profile}`);
  return ['build', '--locked', '-p', 'nomifun-keyboard', '--bin', 'nomifun-keyboard-maintenance',
    ...(target ? ['--target', target] : []), '--profile', profile === 'debug' ? 'dev' : profile, '--message-format=json-render-diagnostics'];
}

export async function buildKeyboardMaintenance(options = {}, spawnCargo = spawn) {
  const child = spawnCargo('cargo', maintenanceBuildArgs(options), {
    cwd: root, env: process.env, stdio: ['inherit', 'pipe', 'inherit'], windowsHide: true,
  });
  const finished = new Promise((accept, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => code === 0 ? accept() : reject(new Error(`Keyboard maintenance build failed: ${code ?? signal}`)));
  });
  finished.catch(() => {});
  let binary;
  for await (const line of createInterface({ input: child.stdout, crlfDelay: Infinity })) {
    let event;
    try { event = JSON.parse(line); } catch { process.stderr.write(line + '\n'); continue; }
    if (event.reason === 'compiler-artifact' && event.target?.name === 'nomifun-keyboard-maintenance' && event.executable) binary = resolve(root, event.executable);
    if (event.message?.rendered) process.stderr.write(event.message.rendered);
  }
  await finished;
  if (!binary) throw new Error('Cargo did not report the keyboard maintenance executable');
  return binary;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const options = {};
  const args = process.argv.slice(2);
  for (let index = 0; index < args.length; index++) {
    if (args[index] === '--target') options.target = args[++index];
    else if (args[index] === '--profile') options.profile = args[++index];
    else throw new Error(`Unknown maintenance build argument: ${args[index]}`);
  }
  process.stdout.write((await buildKeyboardMaintenance(options)) + '\n');
}
