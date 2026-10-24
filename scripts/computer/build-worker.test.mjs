import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { chmod, copyFile, mkdtemp, readFile, rm, stat, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { PassThrough } from 'node:stream';
import path from 'node:path';
import test from 'node:test';
import { buildComputerWorker, cargoWorkerArtifact, workerBuildPlan } from './build-worker.mjs';

function mockCargo(artifact, code = 0) {
  return () => {
    const child = new EventEmitter();
    child.stdout = new PassThrough();
    queueMicrotask(() => {
      child.stdout.end(JSON.stringify({ reason: 'compiler-artifact', target: { name: 'nomifun-computer-worker', kind: ['bin'] }, executable: artifact }) + '\n');
      setImmediate(() => child.emit('close', code, null));
    });
    return child;
  };
}

test('worker target names and independent intermediate directories follow the actual target', () => {
  const root = path.resolve('test-worker-root');
  const plan = workerBuildPlan({ root, target: 'x86_64-pc-windows-msvc', profile: 'debug', environment: {} });
  assert.ok(plan.args.includes(path.join(root, 'apps/computer-worker/Cargo.toml')));
  assert.ok(plan.destination.endsWith('nomifun-computer-worker-x86_64-pc-windows-msvc.exe'));
  assert.equal(plan.environment.CARGO_TARGET_DIR, path.join(root, 'apps/computer-worker/target'));
  assert.ok(plan.environment.CARGO_BUILD_BUILD_DIR.endsWith('build-x86_64-pc-windows-msvc.noindex'));
});

test('caller target/build directories are preserved and non-concrete targets are refused', () => {
  const custom = workerBuildPlan({ target: 'aarch64-apple-darwin', environment: { CARGO_TARGET_DIR: 'custom-output', CARGO_BUILD_BUILD_DIR: 'explicit-intermediates' } });
  assert.equal(custom.environment.CARGO_BUILD_BUILD_DIR, 'explicit-intermediates');
  assert.ok(custom.environment.CARGO_TARGET_DIR.endsWith('custom-output'));
  assert.throws(() => workerBuildPlan({ target: 'universal' }), /concrete Rust target/);
  assert.throws(() => workerBuildPlan({ target: 'x86_64-unknown-linux-gnu', profile: 'unexpected' }), /profile/);
});

test('only the actual worker executable artifact is accepted', () => {
  for (const line of ['diagnostic', '{}', JSON.stringify({ reason: 'compiler-artifact', target: { name: 'other', kind: ['bin'] }, executable: '/wrong' })]) assert.equal(cargoWorkerArtifact(line), null);
  assert.equal(cargoWorkerArtifact(JSON.stringify({ reason: 'compiler-artifact', target: { name: 'nomifun-computer-worker', kind: ['bin'] }, executable: '/actual/nested-target/worker' })), '/actual/nested-target/worker');
});

test('staging uses Cargo-reported nested output and fails before copying on Cargo failure', async () => {
  const copies = [];
  const options = { root: path.resolve('test-worker-root'), target: 'x86_64-unknown-linux-gnu', environment: {} };
  const effects = { spawnCargo: mockCargo('/nested/cargo/artifact'), stage: async (...args) => copies.push(args), createDirectory: async () => {}, executableMode: async () => {} };
  const result = await buildComputerWorker(options, effects);
  assert.equal(copies.length, 1);
  assert.equal(copies[0][0], path.resolve(options.root, '/nested/cargo/artifact'));
  assert.equal(copies[0][1], result.binary);
  await assert.rejects(buildComputerWorker(options, { ...effects, spawnCargo: mockCargo('/nested/cargo/artifact', 101) }), /build failed/);
  assert.equal(copies.length, 1);
});

test('cached worker staging preserves Cargo input timestamps and updates changed bytes', async t => {
  const root = await mkdtemp(path.join(tmpdir(), 'nomifun-worker-staging-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const artifact = path.join(root, 'actual-cargo-artifact');
  await writeFile(artifact, 'worker-one');
  let copies = 0;
  let modeChanges = 0;
  const windows = process.platform === 'win32';
  const options = { root, target: windows ? 'x86_64-pc-windows-msvc' : 'x86_64-unknown-linux-gnu', environment: {} };
  const effects = {
    spawnCargo: mockCargo(artifact),
    stage: async (...args) => { copies++; await copyFile(...args); },
    executableMode: async (...args) => { modeChanges++; await chmod(...args); },
  };
  const first = await buildComputerWorker(options, effects);
  assert.equal(copies, 1); // A missing sidecar must still be staged.
  const oldTimestamp = new Date('2020-01-01T00:00:00Z');
  await utimes(first.binary, oldTimestamp, oldTimestamp);
  const before = await stat(first.binary, { bigint: true });
  const second = await buildComputerWorker(options, effects);
  const after = await stat(second.binary, { bigint: true });
  assert.equal(second.binary, first.binary);
  assert.equal(copies, 1);
  assert.equal(modeChanges, windows ? 0 : 1);
  assert.equal(after.mtimeNs, before.mtimeNs); // Cargo's rerun-if-changed input stays fresh.
  assert.equal(after.ctimeNs, before.ctimeNs); // chmod must not retrigger Tauri's watcher either.

  await writeFile(artifact, 'worker-two'); // Equal size must not hide a real content change.
  await buildComputerWorker(options, effects);
  assert.equal(copies, 2);
  assert.equal(modeChanges, windows ? 0 : 2);
  assert.equal(await readFile(first.binary, 'utf8'), 'worker-two');
  assert.notEqual((await stat(first.binary, { bigint: true })).mtimeNs, before.mtimeNs);
});

test('identical staged worker repairs missing executable permissions without copying', { skip: process.platform === 'win32' }, async t => {
  const root = await mkdtemp(path.join(tmpdir(), 'nomifun-worker-mode-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const artifact = path.join(root, 'actual-cargo-artifact');
  await writeFile(artifact, 'worker');
  const options = { root, target: 'aarch64-apple-darwin', environment: {} };
  const effects = { spawnCargo: mockCargo(artifact) };
  const first = await buildComputerWorker(options, effects);
  await chmod(first.binary, 0o644);
  const before = await stat(first.binary, { bigint: true });
  let modeChanges = 0;
  await buildComputerWorker(options, {
    ...effects,
    stage: async () => assert.fail('identical bytes must not be copied'),
    executableMode: async (...args) => { modeChanges++; await chmod(...args); },
  });
  const after = await stat(first.binary, { bigint: true });
  assert.equal(modeChanges, 1);
  assert.equal(after.mode & 0o777n, 0o755n);
  assert.equal(after.mtimeNs, before.mtimeNs);
});

test('Windows staging never applies Unix executable permissions, including cached workers', async t => {
  const root = await mkdtemp(path.join(tmpdir(), 'nomifun-worker-windows-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const artifact = path.join(root, 'actual-cargo-artifact.exe');
  await writeFile(artifact, 'worker');
  let copies = 0;
  const options = { root, target: 'x86_64-pc-windows-msvc', environment: {} };
  const effects = {
    spawnCargo: mockCargo(artifact),
    stage: async (...args) => { copies++; await copyFile(...args); },
    executableMode: async () => assert.fail('Windows workers do not need chmod'),
  };
  await buildComputerWorker(options, effects);
  await buildComputerWorker(options, effects);
  assert.equal(copies, 1);
  await writeFile(artifact, 'updated-worker');
  await buildComputerWorker(options, effects);
  assert.equal(copies, 2);
});
