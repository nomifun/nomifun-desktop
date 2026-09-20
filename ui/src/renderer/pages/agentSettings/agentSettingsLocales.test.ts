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
  test('does not advertise replacing conversation pages while execution extension authoring remains', () => {
    for (const locale of [en, zh]) {
      expect(Object.hasOwn(locale, 'page')).toBe(false);
      expect(Object.hasOwn(locale, 'view')).toBe(false);
      expect(Object.hasOwn(locale, 'session')).toBe(false);
      expect(locale.middlewareOrder.createBeforeTool.length).toBeGreaterThan(0);
      expect(locale.workbench.configureChatModel.length).toBeGreaterThan(0);
    }
  });
  test('keeps English and Chinese keys in parity', () => {
    expect(flattenKeys(en).sort()).toEqual(flattenKeys(zh).sort());
  });

  test('uses concise official preset names', () => {
    expect(zh.template.chat.minimal.name).toBe('最简');
    expect(zh.template.assistant.general.name).toBe('通用');
    expect(zh.template.coding.codex.name).toBe('编程');
    expect(zh.template.creativeStudio.default.name).toBe('多模');
    expect(en.template.chat.minimal.name).toBe('Minimal');
    expect(en.template.assistant.general.name).toBe('General');
    expect(en.template.coding.codex.name).toBe('Coding');
    expect(en.template.creativeStudio.default.name).toBe('Multimodal');
  });

  test('contains fresh-start copy and no editor test feature in either locale', () => {
    expect(en.freshStart.body.includes('not imported')).toBe(true);
    expect(zh.freshStart.body.includes('不会导入')).toBe(true);
    expect(JSON.stringify(en).includes('Try & inspect')).toBe(false);
    expect(JSON.stringify(zh).includes('试用与检查')).toBe(false);
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

  test('describes Module grants, exact actions and target-owned resource selection', () => {
    expect(en.capabilities.enabled).toBe('Enabled');
    expect(zh.capabilities.enabled).toBe('已启用');
    expect(zh.capabilities.notSelected).toBe('未启用');
    expect(Object.hasOwn(en.capabilities, 'onDemand')).toBe(false);
    expect(en.resources.bindingPolicyBody.includes('conversation')).toBe(true);
    expect(zh.resources.bindingPolicyBody.includes('具体会话')).toBe(true);
    expect(en.workbench.capabilityTab).toBe('Modules & actions');
    expect(zh.workbench.capabilityTab).toBe('模块与操作');
    expect(en.workbench.previewCompileHint.includes('server compile')).toBe(true);
    expect(zh.workbench.previewCompileHint.includes('服务端权威编译')).toBe(true);
  });

  test('contains no Agent Runtime selector or transfer-workbench copy', () => {
    for (const locale of [en, zh]) {
      expect(Object.hasOwn(locale, 'runtimeEngine')).toBe(false);
      expect(Object.hasOwn(locale.workbench, 'moveIn')).toBe(false);
      expect(Object.hasOwn(locale.workbench, 'moveOut')).toBe(false);
      expect(Object.hasOwn(locale.workbench, 'enabledCapabilities')).toBe(false);
      expect(locale.modules.browser.name.length).toBeGreaterThan(0);
      expect(locale.modules.toolDiscovery.name.length).toBeGreaterThan(0);
      expect(locale.effects.destructive.length).toBeGreaterThan(0);
    }
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
