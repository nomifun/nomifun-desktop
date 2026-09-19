import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./desktop-build-mac.sh', import.meta.url), 'utf8');

describe('macOS Desktop build contract', () => {
  test('removes binary import and rejects the retired flag before passthrough', () => {
    expect(source.includes('WITH_CODEX_RUNTIME')).toBe(false);
    expect(source.includes('NOMIFUN_CODEX_RUNTIME')).toBe(false);
    expect(source.includes('stage_runtime_resources')).toBe(false);
    expect(source.includes('stage_runtime_sidecar')).toBe(false);
    expect(source.includes('validate_runtime_hello')).toBe(false);
    expect(source.includes('RUNTIME_STAGE')).toBe(false);
    const rejection = source.indexOf('外部 Codex Runtime 打包入口已移除');
    const passthrough = source.indexOf('PASSTHRU+=("$arg")');
    expect(rejection).toBeGreaterThan(0);
    expect(rejection).toBeLessThan(passthrough);
  });

  test('rejects retired executables and hello metadata in every packaged app', () => {
    expect(source.includes("-name 'nomifun-codex-runtime'")).toBe(true);
    expect(source.includes("-name 'nomifun-codex-runtime.hello.json'")).toBe(true);
    expect(source.includes('app 包含已退役 Codex Runtime 资源')).toBe(true);
    expect(source.includes('无法检查 app 中的已退役 Runtime 资源')).toBe(true);
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
    expect(JSON.parse(overlay).bundle).toEqual({});
  });

  test('stages the pinned arm64 CEF bundle before creating the DMG', () => {
    expect(source.includes('stage-macos-cef-bundle.mjs')).toBe(true);
    expect(source.includes('nomifun-browser-cef-helper')).toBe(true);
    expect(source.includes('cef_macos_aarch64/archive.json')).toBe(true);
    const stage = source.indexOf('stage_macos_cef "$app" "$t"');
    const dmg = source.indexOf('create_dmg_from_staged_app "$app" "$t" "$dmg_dir"');
    const dmgImage = source.indexOf('hdiutil create');
    const dmgSign = source.indexOf('codesign --force --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$output"');
    const notarize = source.indexOf('notarize_dmg_dir "$dmg_dir"');
    expect(stage).toBeGreaterThan(0);
    expect(dmg).toBeGreaterThan(stage);
    expect(dmgSign).toBeGreaterThan(dmgImage);
    expect(notarize).toBeGreaterThan(dmg);
    expect(source.includes('TRIPLES=(aarch64-apple-darwin)')).toBe(true);
  });
});
