import { afterEach, describe, expect, test } from 'bun:test';
import {
  copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync,
  readdirSync, rmSync, utimesSync, writeFileSync,
} from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { pruneIncrementalSessions } from './prune-build.mjs';

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const sandboxes = [];

afterEach(() => {
  for (const sandbox of sandboxes.splice(0)) {
    // These tests never run the cleaner against the checkout's actual caches.
    const target = resolve(sandbox);
    const expectedParent = resolve(tmpdir());
    if (dirname(target) !== expectedParent || !target.startsWith(join(expectedParent, 'nomifun-prune-'))) {
      throw new Error('refusing to remove a directory outside the temporary test sandbox');
    }
    rmSync(target, { recursive: true, force: true });
  }
});

function file(root, name, contents = `sentinel:${name}`) {
  const target = join(root, name);
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, contents);
  return target;
}

function fixture({ activity, working, relocated = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'nomifun-prune-'));
  sandboxes.push(root);
  mkdirSync(join(root, 'scripts', 'lib'), { recursive: true });
  copyFileSync(join(scriptDirectory, 'prune-build.mjs'), join(root, 'scripts', 'prune-build.mjs'));
  copyFileSync(join(scriptDirectory, 'lib', 'build-cleanup-activity.mjs'), join(root, 'scripts', 'lib', 'build-cleanup-activity.mjs'));
  file(root, 'activity.json', JSON.stringify(activity ?? { state: 'idle' }));
  file(root, 'driver.mjs', `
    import { readFileSync } from 'node:fs';
    import { runPruneBuild } from './scripts/prune-build.mjs';
    const activity = JSON.parse(readFileSync(new URL('./activity.json', import.meta.url), 'utf8'));
    const result = runPruneBuild({ inspectActivity: () => {
      if (activity.throw) throw new Error('simulated process inspection failure');
      return activity;
    }});
    console.log('RESULT ' + JSON.stringify(result));
  `);
  const unit = 'build.noindex/debug/incremental/nomifun_companion-hash';
  const old = `${unit}/s-old-abc-cache`;
  const current = `${unit}/s-new-def-cache`;
  file(root, `${old}/sentinel.bin`);
  file(root, `${current}/sentinel.bin`);
  file(root, `${unit}/s-old-abc.lock`, '');
  file(root, `${unit}/s-new-def.lock`, '');
  utimesSync(join(root, old), new Date(1_000), new Date(1_000));
  utimesSync(join(root, current), new Date(2_000), new Date(2_000));
  file(root, 'build.noindex/tmp/keep-until-idle.txt');
  file(root, 'build.noindex/debug/webui-dist/keep-until-idle.txt');
  file(root, 'target/release/bundle/installer.bin');
  if (working) file(root, `${working}/s-active-ghi-working/sentinel.bin`);
  if (relocated) file(root, 'build.noindex/debug/build/tauri-hash/root-output', join(root, 'old-checkout', 'out'));
  return { root, unit, old, current };
}

function snapshot(root) {
  const result = [];
  const visit = directory => {
    for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const target = join(directory, entry.name);
      const name = relative(root, target).split(sep).join('/');
      result.push([name, entry.isDirectory() ? 'directory' : readFileSync(target).toString('base64')]);
      if (entry.isDirectory()) visit(target);
    }
  };
  visit(root);
  return result;
}

function run(root, args = []) {
  const result = spawnSync(process.execPath, [join(root, 'driver.mjs'), ...args], {
    cwd: root, encoding: 'utf8', shell: false, windowsHide: true, timeout: 15_000,
  });
  expect(result.error).toBeUndefined();
  expect(result.status).toBe(0);
  const outcome = result.stdout.split(/\r?\n/).find(line => line.startsWith('RESULT '));
  expect(outcome).toBeDefined();
  return { outcome: JSON.parse(outcome.slice(7)), output: result.stdout + result.stderr };
}

describe('real temporary build-cache cleanup', () => {
  test('active Cargo/compiler/linker keeps the entire tree unchanged before any cleanup branch', () => {
    for (const name of ['cargo', 'rustc', 'rustdoc', 'rust-lld']) {
      const { root } = fixture({ activity: { state: 'busy', names: [name] }, relocated: true });
      const before = snapshot(root);
      const result = run(root, ['--cap', '0']);
      expect(result.outcome).toEqual({ skipped: true, reason: 'active_build' });
      expect(snapshot(root)).toEqual(before);
      expect(result.output).not.toContain('start:');
    }
  });

  test('unknown or throwing process enumeration preserves all sentinels', () => {
    for (const activity of [{ state: 'unknown' }, { throw: true }]) {
      const { root } = fixture({ activity, relocated: true });
      const before = snapshot(root);
      expect(run(root, ['--cap', '0']).outcome).toEqual({ skipped: true, reason: 'unknown_activity' });
      expect(snapshot(root)).toEqual(before);
    }
  });

  test('a working directory skips the full automatic pass, including LRU and hard-cap cleanup', () => {
    for (const working of [
      'build.noindex/debug/incremental/live-unit',
      'target/aarch64-apple-darwin/release/incremental/live-unit',
    ]) {
      const { root } = fixture({ working, relocated: true });
      const before = snapshot(root);
      expect(run(root, ['--cap', '0']).outcome).toEqual({ skipped: true, reason: 'working_session' });
      expect(snapshot(root)).toEqual(before);
    }
  });

  test('pre/post automatic modes also retain unfinished incremental sessions', () => {
    for (const flag of ['--pre', '--post']) {
      const { root } = fixture({ working: 'target/debug/incremental/live-unit' });
      const before = snapshot(root);
      expect(run(root, [flag]).outcome).toEqual({ skipped: true, reason: 'working_session' });
      expect(snapshot(root)).toEqual(before);
    }
  });

  test('explicit clean still refuses active or unverifiable builds', () => {
    for (const activity of [{ state: 'busy', names: ['rustc'] }, { state: 'unknown' }]) {
      const { root } = fixture({ activity, working: 'target/debug/incremental/live-unit' });
      const before = snapshot(root);
      expect(run(root, ['--release']).outcome.skipped).toBe(true);
      expect(snapshot(root)).toEqual(before);
    }
  });

  test('unreadable incremental layout fails closed before any cache changes', () => {
    const { root } = fixture();
    file(root, 'target/debug/incremental', 'not a directory');
    const before = snapshot(root);
    expect(run(root, ['--cap', '0']).outcome).toEqual({ skipped: true, reason: 'unknown_sessions' });
    expect(snapshot(root)).toEqual(before);
  });

  test('a newly observed working session aborts the pass instead of continuing to LRU cleanup', () => {
    const { root, unit } = fixture({ working: 'build.noindex/debug/incremental/nomifun_companion-hash' });
    const before = snapshot(root);
    expect(() => pruneIncrementalSessions(dirname(join(root, unit)))).toThrow();
    expect(snapshot(root)).toEqual(before);
  });

  test('idle cleanup removes old finalized sessions and staging while retaining the newest cache and its lock', () => {
    const { root, old, current, unit } = fixture();
    const result = run(root);
    expect(result.outcome).toEqual({ skipped: false });
    expect(existsSync(join(root, old))).toBe(false);
    expect(existsSync(join(root, unit, 's-old-abc.lock'))).toBe(false);
    expect(readFileSync(join(root, current, 'sentinel.bin'), 'utf8')).toBe(`sentinel:${current}/sentinel.bin`);
    expect(existsSync(join(root, unit, 's-new-def.lock'))).toBe(true);
    expect(existsSync(join(root, 'build.noindex', 'tmp'))).toBe(false);
    expect(existsSync(join(root, 'build.noindex', 'debug', 'webui-dist'))).toBe(false);
    expect(existsSync(join(root, 'target', 'release', 'bundle', 'installer.bin'))).toBe(true);
  });
});
