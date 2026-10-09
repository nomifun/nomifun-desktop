import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./desktop-build-mac.sh', import.meta.url), 'utf8');

describe('macOS Desktop build contract', () => {
  test.skipIf(process.platform !== 'darwin')('rejects flags that bypass complete Browser packaging before invoking Cargo', () => {
    for (const argument of ['--debug', '--no-bundle', '--target=x86_64-apple-darwin', '--profile=custom', '--bundles=dmg']) {
      const result = spawnSync('bash', [fileURLToPath(new URL('./desktop-build-mac.sh', import.meta.url)), '--', argument], { encoding: 'utf8', timeout: 10_000 });
      expect(result.status).toBe(1);
      expect(result.stderr).toContain('完整 macOS release App/DMG');
    }
  });
  test.skipIf(process.platform !== 'darwin')('accepts both macOS targets but rejects an incomplete Universal bundle', () => {
    for (const target of ['arm', 'intel']) {
      const result = spawnSync('bash', [fileURLToPath(new URL('./desktop-build-mac.sh', import.meta.url)), target, '--check'], { encoding: 'utf8', timeout: 10_000 });
      expect(result.status).toBe(0);
    }
    const result = spawnSync('bash', [fileURLToPath(new URL('./desktop-build-mac.sh', import.meta.url)), 'universal', '--check'], { encoding: 'utf8', timeout: 10_000 });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('请分别构建 arm 和 intel');
  });

  test.skipIf(process.platform !== 'darwin')('rejects an invalid DMG format before build tools or compilation', () => {
    const result = spawnSync('bash', [fileURLToPath(new URL('./desktop-build-mac.sh', import.meta.url))], {
      env: { ...process.env, NOMIFUN_MACOS_DMG_FORMAT: 'ULFO' }, encoding: 'utf8', timeout: 10_000,
    });
    expect(result.status).toBe(1);
    expect(result.stderr).toContain('NOMIFUN_MACOS_DMG_FORMAT must be ULMO');
    expect(result.stdout).not.toContain('构建');
    expect(source.indexOf('DMG_FORMAT="$(bun "$DMG_TOOL" format)"')).toBeLessThan(source.indexOf('for tool in bun cargo'));
  });
  test('locks the host/package/legal artifacts without staging external engines', () => {
    expect(source.includes('--sidecar')).toBe(false);
    expect(source.includes('--host "$host"')).toBe(true);
    expect(source.includes('--package "$package"')).toBe(true);
    expect(source.includes('--legal "$license"')).toBe(true);
    expect(source.includes('--legal "$notice"')).toBe(true);
    const overlay = readFileSync(
      new URL('../apps/desktop/tauri.macos.conf.json', import.meta.url), 'utf8',
    );
    expect(JSON.parse(overlay).bundle).toEqual({ macOS: { minimumSystemVersion: '14.0' } });
    const base = JSON.parse(readFileSync(new URL('../apps/desktop/tauri.conf.json', import.meta.url), 'utf8'));
    expect(base.bundle.macOS.minimumSystemVersion).toBe('14.0');
  });

  test('signs the final application before DMG and updater generation', () => {
    const sign = source.indexOf('sign_final_app "$app" "$t"');
    const notarizeApp = source.indexOf('notarize_final_app "$app"');
    const dmg = source.indexOf('create_dmg_from_staged_app "$app" "$t" "$dmg_dir"');
    const updater = source.indexOf('"$APP_BUNDLE_TOOL" updater --root "$ROOT" --app "$app"');
    const notarizeDmg = source.indexOf('notarize_dmg_dir "$dmg_dir"');
    expect(sign).toBeGreaterThan(0);
    expect(notarizeApp).toBeGreaterThan(sign);
    expect(dmg).toBeGreaterThan(notarizeApp);
    expect(updater).toBeGreaterThan(notarizeApp);
    expect(notarizeDmg).toBeGreaterThan(dmg);
    expect(source).toContain('"createUpdaterArtifacts":false');
    expect(source).toContain('codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$output"');
  });

  test('retains Intel ONNX packaging and runtime linkage with no browser download or helpers', () => {
    expect(source).toContain('x86_64-apple-darwin');
    expect(source).toContain('ORT_LIB_PATH="$(bun "$ONNX_RUNTIME_TOOL")"');
    expect(source).toContain('-C link-arg=-Wl,-rpath,@executable_path/../Frameworks');
    expect(source).toContain('onnxruntime-LICENSE');
    expect(source).toContain('onnxruntime-ThirdPartyNotices.txt');
    expect(source).toContain('"$APP_BUNDLE_TOOL" inspect --app "$app"');
    expect(source).not.toMatch(/CEF|cef|browser-runtime|download-cef/);
  });

  test('preserves host website media and local-network privacy reasons', () => {
    const hostPlist = readFileSync(new URL('../apps/desktop/Info.plist', import.meta.url), 'utf8');
    for (const key of ['NSMicrophoneUsageDescription', 'NSCameraUsageDescription', 'NSLocationUsageDescription', 'NSLocalNetworkUsageDescription']) {
      expect(hostPlist).toContain(`<key>${key}</key>`);
    }
  });
});
