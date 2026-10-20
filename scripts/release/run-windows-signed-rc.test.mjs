import { describe, expect, test } from 'bun:test';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';

import {
  assertSelfTest,
  assertSignedRcTargetDirectory,
  signedRcHostSigningCommands,
  signedRcPaths,
  stageArtifacts,
} from './run-windows-signed-rc.mjs';
import { readAndVerifyReleaseLock } from './release-lock.mjs';

function withStagingFixtures(run) {
  const root = mkdtempSync(join(tmpdir(), 'nomifun-rc-staging-test-'));
  try {
    const host = join(root, 'host-input.bin');
    const packagePath = join(root, 'NomiFun-owned_x64-setup.exe');
    // These are ordinary fixture bytes, never executable or signing inputs.
    writeFileSync(host, 'owned Host fixture bytes');
    writeFileSync(packagePath, 'owned package fixture bytes');
    writeFileSync(join(root, 'LICENSE'), 'owned license fixture');
    writeFileSync(join(root, 'NOTICE'), 'owned notice fixture');
    return run({ root, host, packagePath });
  } finally {
    const actual = realpathSync(root);
    if (dirname(actual) !== realpathSync(tmpdir()) || !basename(actual).startsWith('nomifun-rc-staging-test-')) {
      throw new Error('Refuse cleanup outside the exact owned RC staging-test directory');
    }
    rmSync(actual, { recursive: true, force: true });
  }
}

describe('Windows signed RC orchestrator', () => {
  test('uses one immutable source-keyed root and lock path', () => {
    const paths = signedRcPaths('a'.repeat(40), 'C:\\repo');
    expect(paths.root.endsWith('windows-signed-rc\\aaaaaaaaa')).toBe(true);
    expect(paths.lock.endsWith('artifacts\\NomiFun.release-lock.json')).toBe(true);
  });

  test('keeps the no-side-effect self-test runnable', () => {
    expect(assertSelfTest().status).toBe('pass');
  });

  test('signs and verifies the restored RC Host with the configured timestamped identity', () => {
    const host = 'C:\\repo\\target\\x86_64-pc-windows-msvc\\release\\nomifun-desktop.exe';
    expect(signedRcHostSigningCommands(host, {
      WINDOWS_CERTIFICATE_THUMBPRINT: 'a'.repeat(40),
      WINDOWS_TIMESTAMP_URL: 'https://timestamp.example.invalid/tsa',
    })).toEqual([
      ['sign', '/sha1', 'a'.repeat(40), '/fd', 'SHA256', '/tr', 'https://timestamp.example.invalid/tsa', '/td', 'SHA256', host],
      ['verify', '/pa', host],
    ]);
  });

  test('rejects missing signing inputs before any RC build or artifact work', () => {
    for (const [environment, code] of [
      [{}, 'signed_rc_certificate_missing'],
      [{ WINDOWS_CERTIFICATE_THUMBPRINT: 'a'.repeat(40) }, 'signed_rc_timestamp_missing'],
      [{ WINDOWS_CERTIFICATE_THUMBPRINT: 'a'.repeat(40), WINDOWS_TIMESTAMP_URL: '  ' }, 'signed_rc_timestamp_missing'],
    ]) {
      let failure;
      try { signedRcHostSigningCommands('C:\\owned\\host.exe', environment); }
      catch (error) { failure = error; }
      expect(failure?.code).toBe(code);
    }
  });

  test('requires successful Cargo metadata for the exact RC target directory', () => {
    const root = resolve('owned-rc-repository');
    expect(() => assertSignedRcTargetDirectory({
      status: 0, stdout: JSON.stringify({ target_directory: join(root, 'target') }),
    }, root)).not.toThrow();
    for (const result of [
      { status: 1, stdout: '' },
      { status: null, error: { code: 'ENOENT' } },
      { status: 0, stdout: '{invalid' },
      { status: 0, stdout: '{}' },
      { status: 0, stdout: JSON.stringify({ target_directory: join(root, 'other-target') }) },
    ]) expect(() => assertSignedRcTargetDirectory(result, root)).toThrow();
  });

  test('publishes fresh artifact copies with a verified release lock', () => withStagingFixtures(({ root, host, packagePath }) => {
    const source = 'c'.repeat(40);
    const paths = stageArtifacts(source, host, packagePath, root);
    expect(readFileSync(join(paths.artifactRoot, 'nomifun-desktop.exe'))).toEqual(readFileSync(host));
    expect(readFileSync(join(paths.artifactRoot, basename(packagePath)))).toEqual(readFileSync(packagePath));
    for (const legal of ['LICENSE', 'NOTICE']) {
      expect(readFileSync(join(paths.artifactRoot, legal))).toEqual(readFileSync(join(root, legal)));
    }
    const verified = readAndVerifyReleaseLock(paths.lock, { root: paths.root });
    expect(verified.status).toBe('pass');
    expect(verified.lock.source_commit).toBe(source);
    expect(verified.lock.platform).toBe('x86_64-pc-windows-msvc');
    expect(verified.checks.length).toBe(4);
    expect(readdirSync(paths.parent)).toEqual([source.slice(0, 9)]);
  }));

  test('refuses a duplicate RC without overwriting its artifacts or lock', () => withStagingFixtures(({ root, host, packagePath }) => {
    const source = 'd'.repeat(40);
    const paths = stageArtifacts(source, host, packagePath, root);
    const lockBefore = readFileSync(paths.lock);
    const hostBefore = readFileSync(join(paths.artifactRoot, 'nomifun-desktop.exe'));
    writeFileSync(host, 'changed input must never replace published bytes');
    let failure;
    try { stageArtifacts(source, host, packagePath, root); }
    catch (error) { failure = error; }
    expect(failure?.code).toBe('signed_rc_already_exists');
    expect(readFileSync(paths.lock)).toEqual(lockBefore);
    expect(readFileSync(join(paths.artifactRoot, 'nomifun-desktop.exe'))).toEqual(hostBefore);
    expect(readAndVerifyReleaseLock(paths.lock, { root: paths.root }).status).toBe('pass');
    expect(readdirSync(paths.parent)).toEqual([source.slice(0, 9)]);
  }));

  test('cleans only its owned staging directory after artifact preparation fails', () => withStagingFixtures(({ root, host }) => {
    const source = 'e'.repeat(40);
    const paths = signedRcPaths(source, root);
    const neighbor = join(paths.parent, `.${source.slice(0, 9)}.staging-other-owner`);
    mkdirSync(neighbor, { recursive: true });
    writeFileSync(join(neighbor, 'preserve.txt'), 'unrelated owned sentinel');
    expect(() => stageArtifacts(source, host, join(root, 'missing-package.exe'), root)).toThrow();
    expect(existsSync(paths.root)).toBe(false);
    expect(readdirSync(paths.parent)).toEqual([basename(neighbor)]);
    expect(readFileSync(join(neighbor, 'preserve.txt'), 'utf8')).toBe('unrelated owned sentinel');
  }));
});
