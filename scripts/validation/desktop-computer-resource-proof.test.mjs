import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { computerGuideForLifecycle, macosComputerProcessProof } from './desktop-computer-resource-proof.mjs';

const guide = { driver_tool: 'get_guide', effect_class: 'Pure', system_permissions: { accessibility: false, screen_recording: 'never' },
  platforms: { windows: { input_schema: { type: 'object' } }, macos: { input_schema: { type: 'object', required: [] } } } };
const manifest = { tools: [guide] };
const host = { event: 'registered', index: 0, pid: 101, unique_id: '10001', pid_version: 10 };
const worker = { event: 'registered', index: 5, pid: 202, unique_id: '20002', pid_version: 20,
  parent_pid: 101, parent_unique_id: '10001', original_parent_pid_version: 10 };
const exit = { event: 'exit', index: 5, pid: 202, note_exit: true, note_exit_status: true, native_status: 0 };

test('normal lifecycle modes use only their actual permission-free guide schema', () => {
  for (const [platform, mode] of [['win32', 'quit'], ['darwin', 'appkit'], ['darwin', 'cmd-q']]) {
    assert.equal(computerGuideForLifecycle(manifest, platform, mode), guide);
  }
  for (const [platform, mode] of [['win32', 'parent-death'], ['darwin', 'parent-death'], ['darwin', 'sigterm'], ['darwin', 'sigint'], ['linux', 'quit']]) {
    assert.throws(() => computerGuideForLifecycle(manifest, platform, mode));
  }
  const missing = structuredClone(manifest); delete missing.tools[0].platforms.macos;
  assert.throws(() => computerGuideForLifecycle(missing, 'darwin', 'appkit'), /actual frozen schema/);
  const input = structuredClone(manifest); input.tools[0].system_permissions.accessibility = true;
  assert.throws(() => computerGuideForLifecycle(input, 'darwin', 'appkit'));
  const required = structuredClone(manifest); required.tools[0].platforms.macos.input_schema.required = ['topic'];
  assert.throws(() => computerGuideForLifecycle(required, 'darwin', 'appkit'), /empty object/);
});

test('Computer cleanup binds one live direct child to the original App kernel generation', () => {
  assert.deepEqual(macosComputerProcessProof([host, worker], 101, 202), { host, worker });
  for (const changed of [
    { ...worker, parent_pid: 303 }, { ...worker, parent_unique_id: 'stale' },
    { ...worker, original_parent_pid_version: 9 }, { ...worker, unique_id: '0' },
  ]) assert.throws(() => macosComputerProcessProof([host, changed], 101, 202));
  assert.throws(() => macosComputerProcessProof([host, worker, worker], 101, 202));
  assert.throws(() => macosComputerProcessProof([host, worker, exit], 101, 202), /already exited/);
});

test('Computer normal Quit requires exact NOTE_EXIT and zero native status', () => {
  assert.deepEqual(macosComputerProcessProof([host, worker, exit], 101, 202, { exited: true }),
    { host, worker, exit, worker_exit_code: 0 });
  for (const changed of [
    { ...exit, index: 4 }, { ...exit, pid: 203 }, { ...exit, note_exit: false }, { ...exit, note_exit_status: false },
    { ...exit, native_status: 9 }, { ...exit, native_status: 256 }, { ...exit, native_status: undefined },
  ]) assert.throws(() => macosComputerProcessProof([host, worker, changed], 101, 202, { exited: true }));
  assert.throws(() => macosComputerProcessProof([host, worker], 101, 202, { exited: true }));
  assert.throws(() => macosComputerProcessProof([host, worker, exit, exit], 101, 202, { exited: true }));
});

test('native observer distinguishes zero and nonzero exit of an exact owned child', { skip: process.platform !== 'darwin', timeout: 20_000 }, async () => {
  const root = await mkdtemp(join(tmpdir(), 'nomifun-computer-observer-test-'));
  try {
    const observerPath = join(root, 'observer');
    const compiled = spawnSync('/usr/bin/clang', ['-Wall', '-Wextra', '-Werror',
      fileURLToPath(new URL('./macos-owned-process-observer.c', import.meta.url)), '-o', observerPath], { encoding: 'utf8', timeout: 10_000 });
    assert.equal(compiled.status, 0, compiled.stderr);
    for (const code of [0, 7]) {
      const child = spawn(process.execPath, ['-e', `process.stdin.once('data',()=>process.exit(${code}));`], { stdio: ['pipe', 'ignore', 'inherit'] });
      const childExit = new Promise((accept, reject) => { child.once('error', reject); child.once('close', (code, signal) => accept({ code, signal })); });
      const observer = spawn(observerPath, [String(child.pid), await realpath(process.execPath)], { stdio: ['pipe', 'pipe', 'pipe'] });
      let errors = ''; observer.stderr.on('data', bytes => { errors += bytes; });
      const observerExit = new Promise((accept, reject) => { observer.once('error', reject); observer.once('close', (code, signal) => accept({ code, signal })); });
      const frames = [];
      try {
        for await (const line of createInterface({ input: observer.stdout })) {
          const frame = JSON.parse(line); frames.push(frame);
          if (frame.event === 'ready') child.stdin.end('release exact test child\n');
        }
        assert.deepEqual(await observerExit, { code: 0, signal: null }, errors);
        assert.deepEqual(await childExit, { code, signal: null });
        const exits = frames.filter(frame => frame.event === 'exit');
        assert.equal(exits.length, 1); assert.equal(exits[0].pid, child.pid);
        assert.equal(exits[0].note_exit_status, true, 'The kernel must return the requested exit status');
        assert.equal(exits[0].native_status, code << 8);
      } finally {
        if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
        if (observer.exitCode === null && observer.signalCode === null) observer.kill('SIGTERM');
        await Promise.allSettled([childExit, observerExit]);
      }
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});
