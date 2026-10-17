import { describe, expect, test } from 'bun:test';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { MACOS_APPLICATION_ENTITLEMENTS, macosCodeSigningArguments } from './macos-signing.mjs';
import { macosDevelopmentSealArguments } from './macos-dev-signing.mjs';

describe('macOS host resource signing', () => {
  test('declares media and Agent Apple Events access in the shared Tauri entitlements', async () => {
    const plist = await readFile(MACOS_APPLICATION_ENTITLEMENTS, 'utf8');
    expect([...plist.matchAll(/<key>([^<]+)<\/key>/g)].map(match => match[1])).toEqual([
      'com.apple.security.device.audio-input', 'com.apple.security.device.camera',
      'com.apple.security.automation.apple-events',
    ]);
    expect([...plist.matchAll(/<true\/>/g)]).toHaveLength(3);
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

  test.skipIf(process.platform !== 'darwin')('actual ad-hoc Mach-O app signatures retain host entitlements through incremental resealing', async () => {
    // Keep the complete native sequence in an independent CLI event loop.
    // Its real signatures are inspected before and after incremental reseal;
    // native failures retain the fixture and diagnostics for investigation.
    const evidence = await mkdtemp(join(tmpdir(), 'nomifun-signing-report-'));
    try {
      const reportPath = join(evidence, 'report.json');
      const result = spawnSync(process.execPath, [fileURLToPath(new URL('../validation/macos-host-signing-fixture.mjs', import.meta.url)), '--report', reportPath],
        { encoding: 'utf8', timeout: 30_000 });
      expect(result.status, result.stderr).toBe(0);
      // A successful native sequence must publish its exact report; an empty
      // captured stdout or exit 0 alone never turns a no-op into a PASS.
      const report = JSON.parse(await readFile(reportPath, 'utf8'));
      expect(report.passed).toBe(true);
      const expected = {
        'com.apple.security.device.audio-input': true, 'com.apple.security.device.camera': true,
        'com.apple.security.automation.apple-events': true,
      };
      expect(report.initial).toEqual(expected);
      expect(report.resealed).toEqual(expected);
    } finally { await rm(evidence, { recursive: true, force: true }); }
  }, 30_000);
});
