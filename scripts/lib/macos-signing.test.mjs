import { describe, expect, test } from 'bun:test';
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { MACOS_APPLICATION_ENTITLEMENTS, macosCodeSigningArguments } from './macos-signing.mjs';
import { macosDevelopmentSealArguments } from './macos-dev-signing.mjs';
import { signMacosAppBundle } from './macos-app-bundle.mjs';

describe('macOS media resource signing', () => {
  test('declares only existing audio-input and camera access in the shared Tauri entitlements', async () => {
    const plist = await readFile(MACOS_APPLICATION_ENTITLEMENTS, 'utf8');
    expect([...plist.matchAll(/<key>([^<]+)<\/key>/g)].map(match => match[1])).toEqual([
      'com.apple.security.device.audio-input', 'com.apple.security.device.camera',
    ]);
    expect([...plist.matchAll(/<true\/>/g)]).toHaveLength(2);
  });

  test('full and incremental application seals carry media entitlements while nested libraries do not', () => {
    const identity = 'Developer ID Application: Fixture (TEAMFIXTURE)';
    const app = '/fixture/NomiFun.app';
    const full = macosCodeSigningArguments(app, identity, { application: true });
    expect(full).toEqual(['--force', '--options', 'runtime', '--timestamp', '--entitlements', MACOS_APPLICATION_ENTITLEMENTS, '--sign', identity, app]);
    expect(macosDevelopmentSealArguments(app, identity)).toEqual(full);
    expect(macosCodeSigningArguments('/fixture/libonnxruntime.dylib', identity)).not.toContain('--entitlements');
    expect(macosDevelopmentSealArguments(app, '-')).toEqual(['--force', '--entitlements', MACOS_APPLICATION_ENTITLEMENTS, '--sign', '-', app]);
  });

  test.skipIf(process.platform !== 'darwin')('actual ad-hoc Mach-O app signatures retain both entitlements through incremental resealing', async () => {
    const root = await mkdtemp(join(tmpdir(), 'nomifun-media-entitlements-'));
    const app = join(root, 'Fixture.app');
    const program = join(app, 'Contents/MacOS/nomifun-desktop');
    try {
      await mkdir(join(app, 'Contents/MacOS'), { recursive: true });
      await writeFile(join(root, 'fixture.c'), 'int main(void) { return 0; }\n');
      const architecture = process.arch === 'arm64' ? 'arm64' : 'x86_64';
      const compiled = spawnSync('/usr/bin/clang', ['-arch', architecture, '-o', program, join(root, 'fixture.c')], { encoding: 'utf8', timeout: 15_000 });
      expect(compiled.status, compiled.stderr).toBe(0);
      await chmod(program, 0o755);
      await writeFile(join(app, 'Contents/Info.plist'), '<plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.nomifun.media-signing-fixture</string><key>CFBundleExecutable</key><string>nomifun-desktop</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>');
      await signMacosAppBundle({ appPath: app, target: process.arch === 'arm64' ? 'aarch64-apple-darwin' : 'x86_64-apple-darwin' });
      const inspect = () => {
        const dump = join(root, 'signed-entitlements.plist');
        const result = spawnSync('/usr/bin/codesign', ['--display', '--entitlements', dump, '--xml', app], { encoding: 'utf8', timeout: 10_000 });
        expect(result.status, result.stderr).toBe(0);
        const converted = spawnSync('/usr/bin/plutil', ['-convert', 'json', '-o', '-', dump], { encoding: 'utf8', timeout: 10_000 });
        expect(converted.status, converted.stderr).toBe(0);
        expect(JSON.parse(converted.stdout)).toEqual({
          'com.apple.security.device.audio-input': true, 'com.apple.security.device.camera': true,
        });
      };
      inspect();
      const resealed = spawnSync('/usr/bin/codesign', macosDevelopmentSealArguments(app, '-'), { encoding: 'utf8', timeout: 10_000 });
      expect(resealed.status, resealed.stderr).toBe(0);
      expect(spawnSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', app]).status).toBe(0);
      inspect();
    } finally { await rm(root, { recursive: true, force: true }); }
  }, 30_000);
});
