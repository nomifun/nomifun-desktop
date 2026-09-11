import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./release-linux.sh', import.meta.url), 'utf8');
// Run just the actual artifact functions, never the release/auth/upload workflow.
function shellFunction(name) {
  const start = source.indexOf(`${name}() {`);
  if (start < 0) throw new Error(`missing function ${name}`);
  return source.slice(start, source.indexOf('\n}', start) + 2);
}

const linux = ['NomiFun.deb', 'NomiFun.deb.sig', 'NomiFun.deb.release-lock.json',
  'NomiFun.rpm', 'NomiFun.rpm.sig', 'NomiFun.rpm.release-lock.json',
  'NomiFun.AppImage', 'NomiFun.AppImage.sig', 'NomiFun.AppImage.release-lock.json'];
const others = ['NomiFun.dmg', 'NomiFun.release-lock.json', 'NomiFun.app.tar.gz',
  'NomiFun.app.tar.gz.sig', 'NomiFun-setup.exe', 'NomiFun-setup.exe.sig',
  'NomiFun-setup.exe.release-lock.json'];

function fixture(run) {
  const root = mkdtempSync(join(tmpdir(), 'nomifun-linux-release-'));
  const dist = join(root, 'dist/desktop');
  mkdirSync(dist, { recursive: true });
  for (const name of [...linux, ...others]) writeFileSync(join(dist, name), 'fixture');
  try {
    run(root, dist);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

function runFunction(name, root, dist, after = '') {
  return spawnSync('bash', ['-c', `set -euo pipefail
${shellFunction('fail')}
${shellFunction(name)}
bun() { return 0; }
${name}
${after}`], {
    env: { ...process.env, ROOT: root, DistDir: dist }, encoding: 'utf8', timeout: 10_000,
  });
}

describe('Linux release artifact isolation', () => {
  test.skipIf(process.platform !== 'linux')('cleanup preserves macOS/Windows signatures and release locks', () => {
    fixture((root, dist) => {
      const result = runFunction('clean_old_linux_artifacts', root, dist);
      expect(result.status, result.stderr).toBe(0);
      expect(readdirSync(dist).sort()).toEqual([...others].sort());
    });
  });

  test.skipIf(process.platform !== 'linux')('upload selection contains only Linux packages and their evidence', () => {
    fixture((root, dist) => {
      const result = runFunction('collect_assets', root, dist, 'printf "%s\\n" "${Assets[@]}"');
      expect(result.status, result.stderr).toBe(0);
      expect(result.stdout.trim().split('\n').sort()).toEqual(linux.map(name => join(dist, name)).sort());
    });
  });

  for (const [suffix, extension, status] of [
    ['deb', 'deb', 0], ['rpm', 'rpm', 0], ['appimage', 'AppImage', 0], ['deb', 'AppImage', 1],
  ]) {
    test.skipIf(process.platform !== 'linux')(`manifest preflight checks installer format: ${suffix} -> ${extension}`, () => {
      fixture((root, dist) => {
        const key = `linux-x86_64-${suffix}`;
        const out = join(root, 'latest.json');
        writeFileSync(out, JSON.stringify({ version: '0.7.6', platforms: {
          [key]: { url: `https://example.test/releases/download/v0.7.6/NomiFun.${extension}`, signature: 'fixture' },
        } }));
        const validate = source.match(/^validate_manifest\(\) \{[\s\S]*?^NODE\n\}/m)?.[0];
        expect(validate).toBeDefined();
        const result = spawnSync('bash', ['-c', `${validate}\nvalidate_manifest "$ExpectedKey"`], {
          env: { ...process.env, LatestJson: out, TargetVersion: '0.7.6', ExpectedKey: key },
          encoding: 'utf8', timeout: 10_000,
        });
        expect(result.status, result.stderr).toBe(status);
      });
    });
  }
});
