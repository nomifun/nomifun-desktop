import { describe, expect, test } from 'bun:test';
import {
  mkdtempSync,
  mkdirSync,
  rmSync,
  unlinkSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

import {
  createReleaseLock,
  readAndVerifyReleaseLock,
  verifyReleaseLock,
  writeReleaseLock,
} from './release-lock.mjs';

const SOURCE_COMMIT = 'a'.repeat(40);

function fixtureTree() {
  const root = mkdtempSync(join(tmpdir(), 'nomifun-release-lock-'));
  const paths = {
    host: join(root, 'NomiFun.app', 'Contents', 'MacOS', 'nomifun-desktop'),
    sidecar: join(
      root,
      'NomiFun.app',
      'Contents',
      'Resources',
      'runtime',
      'macos',
      'arm64',
      'nomifun-codex-runtime',
    ),
    package: join(root, 'dist', 'NomiFun.dmg'),
    fixture: join(root, 'contracts', 'runtime-release-fixture.json'),
    lock: join(root, 'dist', 'NomiFun.release-lock.json'),
  };
  for (const path of Object.values(paths)) mkdirSync(dirname(path), { recursive: true });
  writeFileSync(paths.host, 'real host');
  writeFileSync(paths.sidecar, 'real sidecar');
  writeFileSync(paths.package, 'real package');
  writeFileSync(paths.fixture, '{"fixture":true}\n');
  return { root, paths };
}

describe('release lock', () => {
  test('hashes only real release artifacts and ignores schema fixtures', () => {
    const { root, paths } = fixtureTree();
    try {
      const lock = createReleaseLock({
        root,
        sourceCommit: SOURCE_COMMIT,
        platform: 'aarch64-apple-darwin',
        host: paths.host,
        sidecars: { macos_desktop_arm64: paths.sidecar },
        packagePath: paths.package,
      });
      expect(Object.keys(lock)).toEqual([
        'schema_version',
        'source_commit',
        'platform',
        'host',
        'sidecars',
        'helpers',
        'package',
        'legal',
      ]);
      expect(lock.helpers).toEqual([]);
      expect(lock.legal).toEqual([]);
      writeReleaseLock(paths.lock, lock);

      expect(readAndVerifyReleaseLock(paths.lock, { root }).status).toBe('pass');

      writeFileSync(paths.fixture, '{"fixture":"changed but irrelevant"}\n');
      expect(readAndVerifyReleaseLock(paths.lock, { root }).status).toBe('pass');

      writeFileSync(paths.sidecar, 'mutated sidecar');
      const mismatch = readAndVerifyReleaseLock(paths.lock, { root });
      expect(mismatch.status).toBe('fail');
      expect(mismatch.checks).toContainEqual(
        expect.objectContaining({
          id: 'sidecars.macos_desktop_arm64',
          status: 'fail',
          reason: 'digest_mismatch',
        }),
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test('reports a missing locked artifact as blocked instead of passing synthetically', () => {
    const { root, paths } = fixtureTree();
    try {
      const lock = createReleaseLock({
        root,
        sourceCommit: SOURCE_COMMIT,
        platform: 'aarch64-apple-darwin',
        host: paths.host,
        sidecars: { macos_desktop_arm64: paths.sidecar },
        packagePath: paths.package,
      });
      unlinkSync(paths.package);
      const result = verifyReleaseLock(lock, { root });
      expect(result.status).toBe('blocked');
      expect(result.checks).toContainEqual(
        expect.objectContaining({
          id: 'package',
          status: 'blocked',
          reason: 'artifact_missing',
        }),
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test('reports a missing release lock as blocked', () => {
    const root = mkdtempSync(join(tmpdir(), 'nomifun-release-lock-missing-'));
    try {
      expect(readAndVerifyReleaseLock(join(root, 'missing.json'), { root })).toEqual(
        expect.objectContaining({
          status: 'blocked',
          reason: 'release_lock_missing',
        }),
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
