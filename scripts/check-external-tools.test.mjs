import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { checkExternalTools, validateComponents } from './check-external-tools.mjs';
const manifest = () => JSON.parse(readFileSync(new URL('../crates/backend/nomifun-runtime/components.json', import.meta.url), 'utf8'));
describe('managed external tool contract', () => {
  test('actual release catalogue, notices and implementation share one contract', () => {
    expect(checkExternalTools().components).toBe(3);
  });
  test('rejects the unrelated same-name Office npm package', () => {
    const value = manifest(); value.components.find(entry => entry.id === 'officecli').package = 'officecli';
    expect(() => validateComponents(value)).toThrow('identity');
  });
  test('keeps Office local and JavaScript engines outside the application payload', () => {
    for (const id of ['bun', 'node', 'officecli']) {
      const value = manifest();
      value.components.find(entry => entry.id === id).delivery = id === 'officecli' ? 'download' : 'embedded';
      expect(() => validateComponents(value)).toThrow('delivery');
    }
  });
  test('rejects an archive from another release and a floating container image', () => {
    const value = manifest(); value.components[0].assets[0].url = 'https://example.com/bun-latest.zip';
    expect(() => validateComponents(value)).toThrow('asset');
    const floating = manifest(); floating.build_images.node = 'node:22';
    expect(() => validateComponents(floating)).toThrow('digest');
  });
});
