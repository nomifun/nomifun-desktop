import { describe, expect, test } from 'bun:test';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import zh from '../../services/i18n/locales/zh-CN/agentSettings.json';

const flattenKeys = (value: unknown, prefix = ''): string[] => {
  if (!value || typeof value !== 'object') return [prefix];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, entry]) =>
    flattenKeys(entry, prefix ? `${prefix}.${key}` : key)
  );
};

describe('Agent Settings locale contract', () => {
  test('keeps English and Chinese keys in parity', () => {
    expect(flattenKeys(en).sort()).toEqual(flattenKeys(zh).sort());
  });

  test('contains fresh-start and real-effect disclosure in both locales', () => {
    expect(en.freshStart.body).toContain('not imported');
    expect(zh.freshStart.body).toContain('不会导入');
    expect(en.test.realEffectWarning).toContain('real resources');
    expect(zh.test.realEffectWarning).toContain('真实资源');
  });
});
