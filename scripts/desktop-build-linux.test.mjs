import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./desktop-build-linux.sh', import.meta.url), 'utf8');

function runBuildFixture(args, noPackageTarget = '') {
  const root = mkdtempSync(join(tmpdir(), 'nomifun-linux-build-'));
  const triple = 'x86_64-unknown-linux-gnu';
  const stale = join(root, 'target', triple, 'release/bundle/rpm/old.rpm');
  const other = join(root, 'target/aarch64-apple-darwin/release/bundle/old.dmg');
  const bin = join(root, 'bin');
  mkdirSync(bin);
  mkdirSync(join(root, 'scripts/release'), { recursive: true });
  copyFileSync(new URL('./desktop-build-linux.sh', import.meta.url), join(root, 'scripts/desktop-build-linux.sh'));
  writeFileSync(join(root, 'scripts/release/release-lock.mjs'), '// mocked');
  for (const path of [stale, other]) {
    mkdirSync(join(path, '..'), { recursive: true });
    writeFileSync(path, 'previous build');
  }
  const tools = {
    'pkg-config': '#!/bin/sh\nexit 0\n',
    rustup: '#!/bin/sh\nprintf "%s\\n" x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu\n',
    bun: `#!/bin/bash
set -eu
if [[ "$1" == x ]]; then
  profile=release
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
      --target) shift; triple="$1" ;;
      --debug|-d) profile=debug ;;
      --profile) shift; profile="$1" ;;
      --profile=*) profile="\${1#--profile=}" ;;
    esac
    shift
  done
  mkdir -p "target/$triple/$profile/bundle/deb"
  touch "target/$triple/$profile/nomifun-desktop"
  chmod +x "target/$triple/$profile/nomifun-desktop"
  if [[ "$triple" != "$MOCK_NO_PACKAGE_TARGET" ]]; then
    printf 'current package' > "target/$triple/$profile/bundle/deb/current-$triple.deb"
  fi
elif [[ "$1" == */release-lock.mjs && "$2" == create ]]; then
  while [[ "$1" != --output ]]; do shift; done
  shift
  printf '{}' > "$1"
fi
`,
  };
  for (const [name, content] of Object.entries(tools)) {
    writeFileSync(join(bin, name), content);
    chmodSync(join(bin, name), 0o755);
  }
  const result = spawnSync('bash', [join(root, 'scripts/desktop-build-linux.sh'), ...args], {
    // Deliberately invoke outside the repository to test root resolution.
    cwd: tmpdir(),
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, MOCK_NO_PACKAGE_TARGET: noPackageTarget },
    encoding: 'utf8',
    timeout: 10_000,
  });
  return { root, stale, other, result };
}

describe('Linux Desktop build contract', () => {
  test('creates and immediately verifies one release lock per collected package', () => {
    expect(source.includes('lock="$package.release-lock.json"')).toBe(true);
    expect(source.includes('write_release_lock "$t" "$host" "$package" "$lock"')).toBe(true);
    expect(source.includes('verify --root "$ROOT" --lock "$output"')).toBe(true);
    expect(source.includes('COLLECTED_LOCKS+=("$lock")')).toBe(true);
  });

  test('locks the native Host, package, and both legal artifacts', () => {
    expect(source.includes('host="$ROOT/target/$t/release/nomifun-desktop"')).toBe(true);
    expect(source.includes('--host "$host"')).toBe(true);
    expect(source.includes('--package "$package"')).toBe(true);
    expect(source.includes('--legal "$ROOT/LICENSE"')).toBe(true);
    expect(source.includes('--legal "$ROOT/NOTICE"')).toBe(true);
  });

  test('fails closed when packaging yields no installable output', () => {
    expect(source.includes('Tauri did not produce any Linux Desktop package')).toBe(true);
  });

  test.skipIf(process.platform !== 'linux')('collects only this build and preserves other platform bundles', () => {
    const fixture = runBuildFixture(['x64', '--', '--bundles', 'deb']);
    try {
      expect(fixture.result.status).toBe(0);
      expect(existsSync(fixture.stale)).toBe(false);
      expect(existsSync(fixture.other)).toBe(true);
      expect(readdirSync(join(fixture.root, 'dist/desktop')).sort()).toEqual([
        'current-x86_64-unknown-linux-gnu.deb',
        'current-x86_64-unknown-linux-gnu.deb.release-lock.json',
      ]);
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  test.skipIf(process.platform !== 'linux')('a previous target cannot hide a later target with no package', () => {
    const fixture = runBuildFixture(['x64', 'arm64'], 'aarch64-unknown-linux-gnu');
    try {
      expect(fixture.result.status).toBe(1);
      expect(fixture.result.stderr).toContain('did not produce any Linux Desktop package for aarch64-unknown-linux-gnu');
    } finally {
      rmSync(fixture.root, { recursive: true, force: true });
    }
  });

  for (const args of [['--debug'], ['-d'], ['--profile', 'dev'], ['--profile=dev'], ['--target', 'aarch64-unknown-linux-gnu']]) {
    test.skipIf(process.platform !== 'linux')(`rejects output-changing flags before deleting Linux bundles: ${args.join(' ')}`, () => {
      const fixture = runBuildFixture(['x64', '--', ...args]);
      try {
        expect(fixture.result.status).toBe(1);
        expect(fixture.result.stderr).toContain('release output');
        expect(existsSync(fixture.stale)).toBe(true);
        expect(existsSync(join(fixture.root, 'dist/desktop'))).toBe(false);
      } finally {
        rmSync(fixture.root, { recursive: true, force: true });
      }
    });
  }
});
