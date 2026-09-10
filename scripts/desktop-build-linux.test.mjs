import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./desktop-build-linux.sh', import.meta.url), 'utf8');

describe('Linux Desktop build contract', () => {
  test('creates and immediately verifies one release lock per collected package', () => {
    expect(source.includes('lock="$package.release-lock.json"')).toBe(true);
    expect(source.includes('write_release_lock "$t" "$host" "$package" "$lock"')).toBe(true);
    expect(source.includes('verify --root "$ROOT" --lock "$output"')).toBe(true);
    expect(source.includes('COLLECTED_LOCKS+=("$lock")')).toBe(true);
  });

  test('locks the native Host, package, and both legal artifacts', () => {
    expect(source.includes('host="$ROOT/target/$t/release/nomifun-desktop"')).toBe(true);
    expect(source.includes('--host "$host"')).toBe(true);
    expect(source.includes('--package "$package"')).toBe(true);
    expect(source.includes('--legal "$ROOT/LICENSE"')).toBe(true);
    expect(source.includes('--legal "$ROOT/NOTICE"')).toBe(true);
  });

  test('fails closed when packaging yields no installable output', () => {
    expect(source.includes('Tauri did not produce any Linux Desktop package')).toBe(true);
  });
});
