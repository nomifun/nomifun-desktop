/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('Preset resource entry points', () => {
  test('shows installed MCP/plugins as explicit opt-in selections', () => {
    const drawer = readSource(new URL('./PresetEditDrawer.tsx', import.meta.url));

    expect(drawer.includes('useMcpServers')).toBe(true);
    expect(drawer.includes('getLoadedExtensions')).toBe(true);
    expect(drawer.includes('selectedMcpResourceKeys')).toBe(true);
    expect(drawer.includes('selectedPluginResourceNames')).toBe(true);
    expect(drawer.includes('setSelectedMcpResourceKeys([])')).toBe(true);
    expect(drawer.includes('setSelectedPluginResourceNames([])')).toBe(true);
    expect(drawer.includes("'preset-installed-mcp-list'")).toBe(true);
    expect(drawer.includes("'preset-installed-plugin-list'")).toBe(true);
    expect(drawer.includes("className='preset-resource-selection-checkbox mt-2px cursor-pointer'")).toBe(true);
    expect(drawer.includes('const resourceSelectionKeys = Array.isArray(selectedKeys) ? selectedKeys : [];')).toBe(true);
    expect(drawer.includes('checked={resourceSelectionKeys.includes(item.key)}')).toBe(true);
    expect(drawer.includes('selectedKeys.includes(item.key)')).toBe(false);
    expect(drawer.includes('toggleSelectedResource(item.key, setSelectedKeys)')).toBe(true);
    expect(drawer.includes('setSelectedMcpResourceKeys(installedMcpItems.map')).toBe(false);
    expect(drawer.includes('setSelectedPluginResourceNames(installedPluginItems.map')).toBe(false);
  });

  test('routes to MCP/plugin markets only after loaded installed lists are empty', () => {
    const drawer = readSource(new URL('./PresetEditDrawer.tsx', import.meta.url));

    expect(drawer.includes("const handleAddMcp = () => navigate('/mcp?tab=market')")).toBe(true);
    expect(drawer.includes("const handleAddPlugin = () => navigate('/mcp?tab=plugin-market')")).toBe(true);
    expect(drawer.includes('!isMcpServersLoading && !hasInstalledMcpServers')).toBe(true);
    expect(drawer.includes('!pluginsLoading && !hasInstalledPlugins')).toBe(true);
    expect(drawer.includes("hasInstalledMcpServers ? '/mcp' : '/mcp?tab=market'")).toBe(false);
    expect(drawer.includes("hasInstalledPlugins ? '/mcp?tab=plugins' : '/mcp?tab=plugin-market'")).toBe(false);
  });

  test('keeps all installed resources selectable in a three-row scroll area', () => {
    const drawer = readSource(new URL('./PresetEditDrawer.tsx', import.meta.url));

    expect(drawer.includes('max-h-[174px] overflow-y-auto')).toBe(true);
    expect(drawer.includes('items.map((item) => (')).toBe(true);
    expect(drawer.includes('items.slice(0, 4)')).toBe(false);
    expect(drawer.includes('settings.presetResourceMore')).toBe(false);
  });

  test('keeps preset explanatory copy under the installed presets tab', () => {
    const page = readSource(new URL('./index.tsx', import.meta.url));
    const list = readSource(new URL('./PresetListPanel.tsx', import.meta.url));
    const zh = readSource(new URL('../../../services/i18n/locales/zh-CN/settings.json', import.meta.url));

    expect(page.includes("subtitle={t('settings.presetsHub.subtitle'")).toBe(false);
    expect(list.includes('settings.presetsListDescription')).toBe(true);
    expect(zh.includes('"presetsListDescription": "将 Agent、模型、Skill 与知识范围固化为可一键启动的复用配置。"')).toBe(true);
  });
});
