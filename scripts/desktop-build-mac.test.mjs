import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./desktop-build-mac.sh', import.meta.url), 'utf8');

// Stop at the Tauri command so this exercises the release shell's actual
// environment forwarding without compiling, downloading, signing or packing.
function releaseCompilerEnvironment(target, flags) {
  const root = mkdtempSync(join(tmpdir(), 'nomifun-macos-release-flags-'));
  try {
    for (const directory of ['scripts/lib', 'scripts/release', 'apps/desktop', 'bin']) {
      mkdirSync(join(root, directory), { recursive: true });
    }
    copyFileSync(new URL('./desktop-build-mac.sh', import.meta.url), join(root, 'scripts/desktop-build-mac.sh'));
    copyFileSync(new URL('./lib/mac-signing.sh', import.meta.url), join(root, 'scripts/lib/mac-signing.sh'));
    writeFileSync(join(root, 'scripts/release/release-lock.mjs'), '// fixture');
    writeFileSync(join(root, 'apps/desktop/tauri.macos.conf.json'), '{}');
    const receipt = join(root, 'environment.json');
    writeFileSync(join(root, 'bin/bun'), `#!${process.execPath}\n` +
      `import { writeFileSync } from 'node:fs';\n` +
      `const args = process.argv.slice(2);\n` +
      `if (args[0] === 'x') {\n` +
      `  writeFileSync(process.env.TEST_RECEIPT, JSON.stringify({\n` +
      `    RUSTFLAGS: process.env.RUSTFLAGS, CARGO_ENCODED_RUSTFLAGS: process.env.CARGO_ENCODED_RUSTFLAGS,\n` +
      `    ORT_LIB_PATH: process.env.ORT_LIB_PATH, ORT_PREFER_DYNAMIC_LINK: process.env.ORT_PREFER_DYNAMIC_LINK,\n` +
      `  })); process.exit(73);\n` +
      `} else if (args.includes('format')) console.log('ULMO');\n` +
      `else if (args.includes('build-settings')) console.log('false');\n` +
      `else if (args[0].endsWith('macos-onnx-runtime.mjs')) console.log('/owned/onnx/lib');\n` +
      `else process.exit(74);\n`, { mode: 0o755 });
    writeFileSync(join(root, 'bin/rustup'), '#!/bin/sh\nprintf "aarch64-apple-darwin\\nx86_64-apple-darwin\\n"\n', { mode: 0o755 });
    const environment = { ...process.env, PATH: `${join(root, 'bin')}:${process.env.PATH}`, TEST_RECEIPT: receipt };
    delete environment.CARGO_ENCODED_RUSTFLAGS;
    Object.assign(environment, flags);
    const result = spawnSync('/bin/bash', [join(root, 'scripts/desktop-build-mac.sh'), target], {
      env: environment, encoding: 'utf8', timeout: 10_000,
    });
    expect(result.status, result.stderr).toBe(73);
    return JSON.parse(readFileSync(receipt, 'utf8'));
  } finally { rmSync(root, { recursive: true, force: true }); }
}

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
  test.skipIf(process.platform !== 'darwin')('forwards Intel ONNX rpath through Cargo\'s active plain or encoded flag source', () => {
    const rpath = 'link-arg=-Wl,-rpath,@executable_path/../Frameworks';
    const plain = releaseCompilerEnvironment('intel', { RUSTFLAGS: '-C debuginfo=1' });
    expect(plain.RUSTFLAGS).toBe(`-C debuginfo=1 -C ${rpath}`);
    expect(plain.CARGO_ENCODED_RUSTFLAGS).toBeUndefined();
    for (const encoded of ['', '-C\x1flink-arg=caller path with spaces', `-C\x1f${rpath}`]) {
      const result = releaseCompilerEnvironment('intel', { RUSTFLAGS: '-C ignored=1', CARGO_ENCODED_RUSTFLAGS: encoded });
      const expected = encoded.includes(rpath) ? encoded : `${encoded}${encoded ? '\x1f' : ''}-C\x1f${rpath}`;
      expect(result.CARGO_ENCODED_RUSTFLAGS).toBe(expected);
      expect(result.ORT_LIB_PATH).toBe('/owned/onnx/lib');
      expect(result.ORT_PREFER_DYNAMIC_LINK).toBe('1');
    }
    const arm = releaseCompilerEnvironment('arm', { RUSTFLAGS: '-C debuginfo=1', CARGO_ENCODED_RUSTFLAGS: '-C\x1flink-arg=caller path' });
    expect(arm.CARGO_ENCODED_RUSTFLAGS).toBe('-C\x1flink-arg=caller path');
    expect(arm.ORT_LIB_PATH).toBeUndefined();
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
    expect(JSON.parse(overlay).bundle).toEqual({ macOS: { minimumSystemVersion: '14.0', entitlements: 'entitlements.plist' } });
    const base = JSON.parse(readFileSync(new URL('../apps/desktop/tauri.conf.json', import.meta.url), 'utf8'));
    expect(base.bundle.macOS.minimumSystemVersion).toBe('14.0');
    expect(base.bundle.macOS.entitlements).toBe('entitlements.plist');
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
    expect(source).toContain('onnx_rpath="link-arg=-Wl,-rpath,@executable_path/../Frameworks"');
    expect(source).toContain('onnxruntime-LICENSE');
    expect(source).toContain('onnxruntime-ThirdPartyNotices.txt');
    expect(source).toContain('"$APP_BUNDLE_TOOL" inspect --app "$app"');
    expect(source).not.toMatch(/CEF|cef|browser-runtime|download-cef/);
  });

  test('preserves host website media and local-network privacy reasons', () => {
    const hostPlist = readFileSync(new URL('../apps/desktop/Info.plist', import.meta.url), 'utf8');
    for (const key of ['NSMicrophoneUsageDescription', 'NSCameraUsageDescription', 'NSLocationUsageDescription', 'NSLocalNetworkUsageDescription', 'NSAppleEventsUsageDescription']) {
      expect(hostPlist).toContain(`<key>${key}</key>`);
    }
  });
});
