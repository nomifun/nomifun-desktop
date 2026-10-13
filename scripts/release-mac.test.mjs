import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, test } from 'bun:test';

function releasePlan(args, authenticated = true) {
  const root = mkdtempSync(join(tmpdir(), 'nomifun-macos-release-plan-'));
  try {
    for (const directory of ['scripts', 'bin', 'apps/desktop/signing']) {
      mkdirSync(join(root, directory), { recursive: true });
    }
    copyFileSync(new URL('./release-mac.sh', import.meta.url), join(root, 'scripts/release-mac.sh'));
    writeFileSync(join(root, 'Cargo.toml'), '[workspace.package]\nversion = "0.7.6"\n');
    // Presence-only dry-run fixtures; these are neither keys nor signing proof.
    writeFileSync(join(root, 'apps/desktop/signing/nomifun-updater.key'), 'fixture');
    writeFileSync(join(root, 'apps/desktop/signing/.env.signing'), '# fixture');
    writeFileSync(join(root, 'apps/desktop/tauri.updater.conf.json'), '{}');
    const receipt = join(root, 'gh-calls.txt');
    writeFileSync(join(root, 'bin/gh'), '#!/bin/sh\n' +
      'printf "%s\\n" "$*" >> "$TEST_GH_RECEIPT"\n' +
      'if [ "$1" = "api" ]; then printf "fixture-user\\n"; fi\n', { mode: 0o755 });
    const result = spawnSync('/bin/bash', [join(root, 'scripts/release-mac.sh'), '-DryRun', ...args], {
      env: { ...process.env, PATH: `${join(root, 'bin')}:${process.env.PATH}`,
        NOMIFUN_RELEASE_TEST_UNAME: 'Darwin', TEST_GH_RECEIPT: receipt },
      encoding: 'utf8', timeout: 10_000,
    });
    const ghCalls = existsSync(receipt) ? readFileSync(receipt, 'utf8') : '';
    expect(ghCalls).toBe(authenticated ? 'api user --jq .login\nrelease view v0.7.6 --repo nomifun/nomifun-desktop --json tagName\n' : '');
    return result;
  } finally { rmSync(root, { recursive: true, force: true }); }
}

describe.skipIf(process.platform === 'win32')('macOS release architecture planning', () => {
  test('defaults to arm64 and plans only the matching native updater and installer', () => {
    const result = releasePlan([]);
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toContain('aarch64-apple-darwin (darwin-aarch64)');
    expect(result.stdout).toContain('NomiFun_0.7.6_aarch64.dmg');
    expect(result.stdout).toContain('target/aarch64-apple-darwin/release/bundle/macos/NomiFun.app.tar.gz');
    expect(result.stdout).not.toContain('删除旧 Intel 条目');
  });

  test.each(['intel', 'x86_64-apple-darwin'])('plans %s through its matching Intel installer and updater', architecture => {
    const result = releasePlan(['-Architecture', architecture]);
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toContain('x86_64-apple-darwin (darwin-x86_64)');
    expect(result.stdout).toContain('NomiFun_0.7.6_x64.dmg');
    expect(result.stdout).toContain('target/x86_64-apple-darwin/release/bundle/macos/NomiFun_0.7.6_x64.app.tar.gz');
  });

  test('retires an Intel updater entry only when explicitly selected by the caller', () => {
    const result = releasePlan(['--macos-arm64-only']);
    expect(result.status, result.stderr).toBe(0);
    expect(result.stdout).toContain('显式仅保留 Apple Silicon，删除旧 Intel 条目');
    const source = readFileSync(new URL('./release-mac.sh', import.meta.url), 'utf8');
    expect(source).toContain('bun run build:mac --signed "$Architecture" --config "$UpdaterConf"');
    expect(source).toContain('if [[ "$MacosArm64Only" -eq 1 ]]; then\n  LatestArgs+=(--macos-arm64-only)');
  });

  test.each([
    [['-Architecture', 'universal'], '未知 macOS 架构'],
    [['-Architecture', 'intel', '--macos-arm64-only'], 'Intel 发布不能使用 --macos-arm64-only'],
  ])('rejects unsupported architecture selection %j before authentication or publishing', (args, message) => {
    const result = releasePlan(args, false);
    expect(result.status).toBe(1);
    expect(result.stderr).toContain(message);
  });
});
