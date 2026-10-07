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
    helper: join(
      root,
      'NomiFun.app',
      'Contents',
      'Resources',
      'helpers',
      'native-helper',
    ),
    package: join(root, 'dist', 'NomiFun.dmg'),
    fixture: join(root, 'contracts', 'schema-fixture.json'),
    lock: join(root, 'dist', 'NomiFun.release-lock.json'),
  };
  for (const path of Object.values(paths)) mkdirSync(dirname(path), { recursive: true });
  writeFileSync(paths.host, 'real host');
  writeFileSync(paths.helper, 'real helper');
  writeFileSync(paths.package, 'real package');
  writeFileSync(paths.fixture, '{"fixture":true}\n');
  return { root, paths };
}

describe('release lock', () => {
  test('creates and verifies a host and package lock without sidecars', () => {
    const { root, paths } = fixtureTree();
    try {
      const lock = createReleaseLock({
        root,
        sourceCommit: SOURCE_COMMIT,
        platform: 'x86_64-pc-windows-msvc',
        host: paths.host,
        packagePath: paths.package,
      });

      expect(lock.schema_version).toBe('2.0.0');
      expect(Object.hasOwn(lock, 'sidecars')).toBe(false);
      expect(verifyReleaseLock(lock, { root })).toEqual(
        expect.objectContaining({
          status: 'pass',
          checks: [
            expect.objectContaining({ id: 'host', status: 'pass' }),
            expect.objectContaining({ id: 'package', status: 'pass' }),
          ],
        }),
      );

      writeReleaseLock(paths.lock, lock);
      expect(readAndVerifyReleaseLock(paths.lock, { root }).status).toBe('pass');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  test('hashes only real release artifacts and ignores schema fixtures', () => {
    const { root, paths } = fixtureTree();
    try {
      const lock = createReleaseLock({
        root,
        sourceCommit: SOURCE_COMMIT,
        platform: 'aarch64-apple-darwin',
        host: paths.host,
        helpers: [paths.helper],
        packagePath: paths.package,
      });
      expect(Object.keys(lock)).toEqual([
        'schema_version',
        'source_commit',
        'platform',
        'host',
        'helpers',
        'package',
        'legal',
      ]);
      expect(lock.helpers).toHaveLength(1);
      expect(lock.legal).toEqual([]);
      writeReleaseLock(paths.lock, lock);

      expect(readAndVerifyReleaseLock(paths.lock, { root }).status).toBe('pass');

      writeFileSync(paths.fixture, '{"fixture":"changed but irrelevant"}\n');
      expect(readAndVerifyReleaseLock(paths.lock, { root }).status).toBe('pass');

      writeFileSync(paths.helper, 'mutated helper');
      const mismatch = readAndVerifyReleaseLock(paths.lock, { root });
      expect(mismatch.status).toBe('fail');
      expect(mismatch.checks).toContainEqual(
        expect.objectContaining({
          id: 'helpers[0]',
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

  test('does not silently convert retired sidecar inputs or locks', () => {
    const { root, paths } = fixtureTree();
    try {
      const input = {
        root,
        sourceCommit: SOURCE_COMMIT,
        platform: 'aarch64-apple-darwin',
        host: paths.host,
        packagePath: paths.package,
      };
      expect(() => createReleaseLock({ ...input, sidecars: {} }))
        .toThrow('unsupported release-lock inputs');
      const current = createReleaseLock(input);
      const legacy = { ...current, schema_version: '1.0.0', sidecars: {} };
      expect(verifyReleaseLock(legacy, { root })).toEqual(expect.objectContaining({
        status: 'fail', reason: 'invalid_release_lock',
      }));
      expect(verifyReleaseLock({ ...current, sidecars: {} }, { root })).toEqual(
        expect.objectContaining({ status: 'fail', reason: 'invalid_release_lock' }),
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
