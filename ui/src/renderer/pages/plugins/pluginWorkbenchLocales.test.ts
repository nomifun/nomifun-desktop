/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import en from '@/renderer/services/i18n/locales/en-US/pluginWorkbench.json';
import zh from '@/renderer/services/i18n/locales/zh-CN/pluginWorkbench.json';

const flattenKeys = (value: unknown, prefix = ''): string[] => {
  if (!value || typeof value !== 'object') return [prefix];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, entry]) =>
    flattenKeys(entry, prefix ? `${prefix}.${key}` : key)
  );
};

describe('Plugin Workbench locale contract', () => {
  test('keeps English and Chinese keys in parity', () => {
    expect(flattenKeys(en).sort()).toEqual(flattenKeys(zh).sort());
  });

  test('uses an unambiguous workbench label', () => {
    expect(zh.title).toBe('Plugin 工作台');
    expect(zh.navigation.railTitle).toBe('Plugin 工作台');
    expect(en.title).toBe('Plugin Workbench');
    expect(en.navigation.railTitle).toBe('Plugin Workbench');
  });

  test('states retained-data deletion separately from uninstall', () => {
    expect(zh.confirm.uninstallBody.includes('保留')).toBe(true);
    expect(zh.confirm.deleteDataBody.includes('删除')).toBe(true);
    expect(en.confirm.uninstallBody.includes('remain')).toBe(true);
    expect(en.confirm.deleteDataBody.includes('removes')).toBe(true);
  });

  test('contains the complete authoring workflow and destructive confirmation copy', () => {
    expect(zh.actions.createProject).toBeTruthy();
    expect(zh.dialogs.create.title).toBeTruthy();
    expect(zh.dialogs.import.title).toBeTruthy();
    expect(zh.dialogs.test.sideEffectWarning.includes('副作用')).toBe(true);
    expect(zh.confirm.deleteProjectBody.includes('已安装 Plugin')).toBe(true);
    expect(en.workflow.apply).toBe('Apply');
    expect(en.confirm.deleteProjectBody.includes('installed Plugin')).toBe(true);
  });
});
