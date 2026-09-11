import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { describe, expect, test } from 'bun:test';

function generate(packages, previous) {
  const root = mkdtempSync(join(tmpdir(), 'nomifun-linux-updater-'));
  try {
    mkdirSync(join(root, 'scripts'));
    copyFileSync(new URL('./make-latest-json.mjs', import.meta.url), join(root, 'scripts/make-latest-json.mjs'));
    const out = join(root, 'latest.json');
    if (previous) writeFileSync(out, JSON.stringify(previous));
    for (const [triple, name] of packages) {
      const path = join(root, 'target', triple, 'release/bundle', name);
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, `fixture package: ${name}`);
      // Routing tests, not cryptographic signing/verification evidence.
      writeFileSync(`${path}.sig`, `fixture-signature:${name}`);
    }
    const result = spawnSync(process.execPath, [join(root, 'scripts/make-latest-json.mjs'),
      '--version', '0.7.6', '--out', out, '--notes', 'Linux fixture', '--collect'], {
      encoding: 'utf8', timeout: 10_000,
    });
    expect(result.status, result.stderr).toBe(0);
    return JSON.parse(readFileSync(out, 'utf8'));
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

// Mirrors tauri-plugin-updater 2.10.1 get_urls: installer-qualified target first,
// generic OS/arch target second. The matching installer checks the package bytes.
function updaterEntry(manifest, arch, installer) {
  return manifest.platforms[`linux-${arch}-${installer}`] ?? manifest.platforms[`linux-${arch}`];
}

describe('Linux updater installer routing', () => {
  for (const [triple, arch] of [
    ['x86_64-unknown-linux-gnu', 'x86_64'],
    ['aarch64-unknown-linux-gnu', 'aarch64'],
  ]) {
    test(`${arch}: installed deb/rpm/AppImage receive their own package and signature`, () => {
      const packages = ['deb', 'rpm', 'AppImage'].map(ext => [triple, `NomiFun_0.7.6_${arch}.${ext}`]);
      const manifest = generate(packages);
      for (const [, name] of packages) {
        const installer = name.split('.').at(-1).toLowerCase();
        const entry = updaterEntry(manifest, arch, installer);
        expect(entry.url).toEndWith(`/${name}`);
        expect(entry.signature).toBe(`fixture-signature:${name}`);
      }
      expect(manifest.platforms[`linux-${arch}`]).toEqual(manifest.platforms[`linux-${arch}-appimage`]);
    });
  }

  test('deb/rpm-only builds do not advertise an incompatible generic AppImage fallback', () => {
    const manifest = generate([
      ['x86_64-unknown-linux-gnu', 'NomiFun_0.7.6_amd64.deb'],
      ['x86_64-unknown-linux-gnu', 'NomiFun_0.7.6_x86_64.rpm'],
    ]);
    expect(updaterEntry(manifest, 'x86_64', 'appimage')).toBeUndefined();
    expect(updaterEntry(manifest, 'x86_64', 'deb').url).toEndWith('.deb');
    expect(updaterEntry(manifest, 'x86_64', 'rpm').url).toEndWith('.rpm');
  });

  test('same-version append preserves other platforms and already published Linux formats', () => {
    const previous = {
      version: '0.7.6', platforms: {
        'darwin-aarch64': { url: 'https://example.test/mac.app.tar.gz', signature: 'mac' },
        'windows-x86_64': { url: 'https://example.test/win.exe', signature: 'win' },
        'linux-x86_64': { url: 'https://example.test/old.AppImage', signature: 'appimage' },
        'linux-x86_64-rpm': { url: 'https://example.test/old.rpm', signature: 'rpm' },
      },
    };
    const manifest = generate([['x86_64-unknown-linux-gnu', 'NomiFun_0.7.6_amd64.deb']], previous);
    for (const [key, entry] of Object.entries(previous.platforms)) {
      expect(manifest.platforms[key]).toEqual(entry);
    }
    expect(manifest.platforms['linux-x86_64-deb'].url).toEndWith('.deb');
  });

  test('macOS and Windows generation retains its existing keys and package choices', () => {
    const manifest = generate([
      ['x86_64-unknown-linux-gnu', 'NomiFun_0.7.6_amd64.deb'],
      ['universal-apple-darwin', 'NomiFun.app.tar.gz'],
      ['x86_64-pc-windows-msvc', 'NomiFun-setup.exe'],
      ['x86_64-pc-windows-msvc', 'NomiFun.msi'],
    ]);
    expect(Object.keys(manifest.platforms).sort()).toEqual([
      'darwin-aarch64', 'darwin-x86_64', 'linux-x86_64-deb', 'windows-x86_64',
    ]);
    expect(manifest.platforms['darwin-aarch64'].url).toEndWith('/NomiFun.app.tar.gz');
    expect(manifest.platforms['darwin-x86_64']).toEqual(manifest.platforms['darwin-aarch64']);
    expect(manifest.platforms['windows-x86_64'].url).toEndWith('/NomiFun-setup.exe');
  });
});
