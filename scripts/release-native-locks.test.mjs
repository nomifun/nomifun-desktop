import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const mac = readFileSync(new URL('./release-mac.sh', import.meta.url), 'utf8');
const linux = readFileSync(new URL('./release-linux.sh', import.meta.url), 'utf8');

describe('native release evidence publishing', () => {
  test('macOS verifies and uploads the DMG release lock', () => {
    expect(mac.includes('ReleaseLock="${Dmg%.dmg}.release-lock.json"')).toBe(true);
    expect(mac.includes('release-lock.mjs verify --root "$ROOT" --lock "$ReleaseLock"')).toBe(true);
    expect(mac.match(/release (?:create|upload)[^\n]+"\$ReleaseLock"/g)).toHaveLength(2);
  });

  test('macOS release uses the supported arm64 packages and scans only its current target', () => {
    expect(mac.includes('Triple="aarch64-apple-darwin"')).toBe(true);
    expect(mac.includes('NomiFun_${TargetVersion}_aarch64.dmg')).toBe(true);
    expect(mac.includes('--target-dir "target/$Triple" --target-triple "$Triple" --macos-arm64-only')).toBe(true);
    expect(mac.includes("['darwin-x86_64', 'darwin-aarch64']")).toBe(false);
  });

  test('Linux requires, verifies, and uploads one release lock per package', () => {
    expect(linux.includes('lock="$package.release-lock.json"')).toBe(true);
    expect(linux.includes('release-lock.mjs verify --root "$ROOT" --lock "$lock"')).toBe(true);
    for (const format of ['deb', 'AppImage', 'rpm']) {
      expect(linux.includes(`-name '*.${format}.release-lock.json'`)).toBe(true);
    }
  });
});
