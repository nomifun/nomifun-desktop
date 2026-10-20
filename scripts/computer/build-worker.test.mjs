import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import path from 'node:path';
import test from 'node:test';
import { buildComputerWorker, cargoWorkerArtifact, workerBuildPlan } from './build-worker.mjs';

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
  const mock = code => () => {
    const child = new EventEmitter();
    child.stdout = new PassThrough();
    queueMicrotask(() => {
      child.stdout.end(JSON.stringify({ reason: 'compiler-artifact', target: { name: 'nomifun-computer-worker', kind: ['bin'] }, executable: '/nested/cargo/artifact' }) + '\n');
      setImmediate(() => child.emit('close', code, null));
    });
    return child;
  };
  const options = { root: path.resolve('test-worker-root'), target: 'x86_64-unknown-linux-gnu', environment: {} };
  const effects = { spawnCargo: mock(0), stage: async (...args) => copies.push(args), createDirectory: async () => {}, executableMode: async () => {} };
  const result = await buildComputerWorker(options, effects);
  assert.equal(copies.length, 1);
  assert.equal(copies[0][0], path.resolve(options.root, '/nested/cargo/artifact'));
  assert.equal(copies[0][1], result.binary);
  await assert.rejects(buildComputerWorker(options, { ...effects, spawnCargo: mock(101) }), /build failed/);
  assert.equal(copies.length, 1);
});
