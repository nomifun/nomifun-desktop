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
      expect(result.stderr).toContain('完整 arm64 release App/DMG');
    }
  });
  test('keeps the context-only shutdown probe separate from navigation and soak', () => {
    const runner = readFileSync(new URL('./validation/run-macos-cef-smoke.mjs', import.meta.url), 'utf8');
    const fixture = readFileSync(new URL('../apps/desktop/examples/browser_cef_smoke.rs', import.meta.url), 'utf8');
    expect(runner.includes("args.includes('--context-shutdown-only')")).toBe(true);
    expect(runner.includes("'NOMIFUN_CEF_CONTEXT_SHUTDOWN_ONLY=1'")).toBe(true);
    expect(runner.includes("'tauri-native-cef-context-shutdown'")).toBe(true);
    expect(runner.includes('productAcceptance: false')).toBe(true);
    const start = fixture.indexOf('if context_shutdown_only {');
    const branch = fixture.slice(start, fixture.indexOf('if window_reopen_only {', start));
    expect(branch.includes('engine.create_context(')).toBe(true);
    expect(branch.includes('create_page(')).toBe(false);
    expect(branch.includes('retained_shutdown_context = Some')).toBe(true);
    expect(fixture.indexOf('drop(retained_shutdown_context)')).toBeGreaterThan(fixture.indexOf('let shutdown = engine.shutdown().await'));
  });

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
    expect(source.includes('"$BROWSER_BUNDLE_TOOL" runtime')).toBe(true);
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

  test('generates updater bytes only after CEF staging and final App notarization', () => {
    const stage = source.indexOf('stage_macos_cef "$app" "$t"');
    const notarizeApp = source.indexOf('notarize_final_app "$app"');
    const updater = source.indexOf('"$BROWSER_BUNDLE_TOOL" updater --root "$ROOT" --app "$app"');
    expect(source.includes('"createUpdaterArtifacts":false')).toBe(true);
    expect(notarizeApp).toBeGreaterThan(stage);
    expect(updater).toBeGreaterThan(notarizeApp);
  });

  test('declares website media and local-network privacy reasons in the host and staged CEF helpers', () => {
    const hostPlist = readFileSync(
      new URL('../apps/desktop/Info.plist', import.meta.url), 'utf8',
    );
    const stage = readFileSync(
      new URL('./lib/macos-browser-bundle.mjs', import.meta.url), 'utf8',
    );
    const smoke = readFileSync(
      new URL('./validation/run-macos-cef-smoke.mjs', import.meta.url), 'utf8',
    );
    for (const key of [
      'NSMicrophoneUsageDescription',
      'NSCameraUsageDescription',
      'NSLocationUsageDescription',
      'NSLocalNetworkUsageDescription',
    ]) {
      expect(hostPlist.includes(`<key>${key}</key>`)).toBe(true);
      expect(stage.includes(`${key}:`)).toBe(true);
      expect(smoke.includes(`${key}:`)).toBe(true);
    }
  });

  test('canonicalizes host and helper Info.plists before CEF bundle signing', () => {
    const stage = readFileSync(
      new URL('./lib/macos-browser-bundle.mjs', import.meta.url), 'utf8',
    );
    const hostCanonicalization = stage.indexOf(
      "await canonicalizeInfoPlist(join(contents, 'Info.plist'));",
    );
    const helperCanonicalization = stage.indexOf(
      "await canonicalizeInfoPlist(join(frameworks, `${name}.app`, 'Contents/Info.plist'));",
    );
    const helperSigning = stage.indexOf(
      'for (const name of helperNames) await sign(join(frameworks, `${name}.app`), true);',
    );
    const appSigning = stage.indexOf('await sign(app);');
    expect(stage.includes("run('/usr/bin/plutil', ['-convert', 'xml1', path])")).toBe(true);
    expect(hostCanonicalization).toBeGreaterThan(0);
    expect(helperCanonicalization).toBeGreaterThan(hostCanonicalization);
    expect(helperCanonicalization).toBeLessThan(helperSigning);
    expect(hostCanonicalization).toBeLessThan(appSigning);
    expect(stage.includes("info_plist_serialization: 'canonical-xml'")).toBe(true);
  });

  test('keeps the native CEF 100-cycle soak isolated and fail-closed', () => {
    const runner = readFileSync(
      new URL('./validation/run-macos-cef-smoke.mjs', import.meta.url), 'utf8',
    );
    const fixture = readFileSync(
      new URL('../apps/desktop/examples/browser_cef_smoke.rs', import.meta.url), 'utf8',
    );
    expect(runner.includes("args.includes('--soak-only')")).toBe(true);
    expect(runner.includes("'NOMIFUN_CEF_SOAK_ONLY=1'")).toBe(true);
    expect(runner.includes("infoPlistSerialization: 'canonical-xml'")).toBe(true);
    expect(fixture.includes('for cycle in 0..100usize')).toBe(true);
    expect(fixture.includes('Err(WorkspaceError::StaleTarget)')).toBe(true);
    expect(fixture.includes('stale_target_rejections == 99')).toBe(true);
    expect(fixture.includes('latency_not_sequence_degraded')).toBe(true);
    expect(fixture.includes('let close = runtime.close().await')).toBe(true);
    expect(fixture.includes('let shutdown = engine.shutdown().await')).toBe(true);
  });

  test('keeps native CEF window replacement identities and pending-work fences explicit', () => {
    const runner = readFileSync(
      new URL('./validation/run-macos-cef-smoke.mjs', import.meta.url), 'utf8',
    );
    const fixture = readFileSync(
      new URL('../apps/desktop/examples/browser_cef_smoke.rs', import.meta.url), 'utf8',
    );
    const lifecycle = readFileSync(
      new URL('../apps/desktop/examples/support/browser_window_reopen.rs', import.meta.url), 'utf8',
    );
    expect(runner.includes("args.includes('--window-reopen-only')")).toBe(true);
    expect(runner.includes("'NOMIFUN_CEF_WINDOW_REOPEN_ONLY=1'")).toBe(true);
    expect(fixture.includes('mod browser_window_reopen;')).toBe(true);
    expect(lifecycle.includes('BrowserEvaluationOutcome::AwaitingDialog')).toBe(true);
    expect(lifecycle.includes('Err(WorkspaceError::WorkspaceClosed)')).toBe(true);
    expect(lifecycle.includes('Err(WorkspaceError::TabNotFound)')).toBe(true);
    expect(lifecycle.includes('old_window_identity == new_window_identity')).toBe(true);
    expect(lifecycle.includes('create_runtime(engine, app, base_url, 52, "new")')).toBe(true);
  });
});
