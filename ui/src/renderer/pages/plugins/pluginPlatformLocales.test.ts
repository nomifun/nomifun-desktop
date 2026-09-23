import { expect, test } from 'bun:test';
import en from '@/renderer/services/i18n/locales/en-US/pluginPlatform.json';
import zh from '@/renderer/services/i18n/locales/zh-CN/pluginPlatform.json';

function keys(value: unknown, prefix = ''): string[] {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return [prefix];
  return Object.entries(value).flatMap(([key, child]) => keys(child, prefix ? `${prefix}.${key}` : key));
}

test('Unified Plugin locale keys stay equal and omit retired product language', () => {
  expect(keys(en).sort()).toEqual(keys(zh).sort());
  const encoded = JSON.stringify({ en, zh });
  for (const retired of [
    'Ready Candidate', 'Ready Release', 'Auto Apply', 'Auto Publish',
    'Plugin Mount', 'Plugin Product', 'Plugin Project', 'MiniApp',
  ]) expect(encoded).not.toContain(retired);
});
