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
    expect(en.freshStart.body.includes('not imported')).toBe(true);
    expect(zh.freshStart.body.includes('不会导入')).toBe(true);
    expect(en.test.realEffectWarning.includes('not simulated')).toBe(true);
    expect(zh.test.realEffectWarning.includes('不会模拟')).toBe(true);
  });

  test('uses Agent Workbench as the sole public authoring label', () => {
    expect(en.title).toBe('Agent Workbench');
    expect(zh.title).toBe('Agent 工作台');
    expect(en.navigation.railTitle).toBe('Agent Workbench');
    expect(zh.navigation.railTitle).toBe('Agent 工作台');
    expect(Object.hasOwn(en.navigation, 'entryDescription')).toBe(false);
    expect(Object.hasOwn(en.navigation, 'open')).toBe(false);
    expect(Object.hasOwn(zh.navigation, 'entryDescription')).toBe(false);
    expect(Object.hasOwn(zh.navigation, 'open')).toBe(false);
  });

  test('describes capability modes and target-owned resource selection', () => {
    expect(en.capabilities.enabled).toBe('Enabled');
    expect(zh.capabilities.enabled).toBe('已启用');
    expect(zh.capabilities.notSelected).toBe('未启用');
    expect(Object.hasOwn(en.capabilities, 'onDemand')).toBe(false);
    expect(en.resources.bindingPolicyBody.includes('conversation')).toBe(true);
    expect(zh.resources.bindingPolicyBody.includes('具体会话')).toBe(true);
  });

  test('does not expose the removed Typed resources product term', () => {
    expect(JSON.stringify(en).toLowerCase().includes('typed resource')).toBe(false);
    expect(JSON.stringify(zh).toLowerCase().includes('typed resource')).toBe(false);
  });

  test('contains localized user Agent deletion copy', () => {
    expect(en.library.deleteConfirmTitle.includes('{{name}}')).toBe(true);
    expect(zh.library.deleteConfirmTitle.includes('{{name}}')).toBe(true);
    expect(en.library.deleteConfirmBody.includes('history')).toBe(true);
    expect(zh.library.deleteConfirmBody.includes('历史')).toBe(true);
  });
});
