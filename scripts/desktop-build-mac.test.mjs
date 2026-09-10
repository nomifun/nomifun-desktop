import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./desktop-build-mac.sh', import.meta.url), 'utf8');

describe('macOS Desktop build contract', () => {
  test('keeps the retired Codex Runtime sidecar explicitly opt-in', () => {
    expect(source.includes('WITH_CODEX_RUNTIME=0')).toBe(true);
    expect(source.includes('--with-codex-runtime')).toBe(true);
    expect(source.includes('if [[ "$WITH_CODEX_RUNTIME" -eq 0 ]]; then')).toBe(true);
    expect(source.includes('当前 Nomi-core 构建不包含旧 Codex Runtime sidecar')).toBe(true);
  });

  test('fails if a default Nomi-core app accidentally packages the legacy sidecar', () => {
    expect(source.includes("-name 'nomifun-codex-runtime'")).toBe(true);
    expect(source.includes('默认 Nomi-core app 意外包含旧 Codex Runtime sidecar')).toBe(true);
  });

  test('adds sidecars to the release lock only in explicit compatibility mode', () => {
    expect(
      source.match(/\[\[ "\$WITH_CODEX_RUNTIME" -eq 1 \]\]/g)?.length,
    ).toBeGreaterThanOrEqual(4);
    expect(source.includes('--sidecar "macos_desktop_arm64=')).toBe(true);
    expect(source.includes('--sidecar "macos_desktop_x64=')).toBe(true);
  });
});
