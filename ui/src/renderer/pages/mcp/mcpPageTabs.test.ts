/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('MCP hub tab structure', () => {
  test('separates installed and market tabs for MCP and plugins', () => {
    const page = readSource(new URL('./index.tsx', import.meta.url));
    const mcpMarket = readSource(new URL('./McpMarketSettings.tsx', import.meta.url));
    const plugins = readSource(new URL('./PluginSettingsPanel.tsx', import.meta.url));
    const zh = readSource(new URL('../../services/i18n/locales/zh-CN/settings.json', import.meta.url));

    expect(page.includes("type McpTab = 'servers' | 'market' | 'plugins' | 'plugin-market'")).toBe(true);
    expect(page.includes("title={t('settings.mcpPage.installedMcpTab'")).toBe(true);
    expect(page.includes("title={t('settings.mcpPage.mcpMarketTab'")).toBe(true);
    expect(page.includes("title={t('settings.mcpPage.installedPluginsTab'")).toBe(true);
    expect(page.includes("title={t('settings.mcpPage.pluginMarketTab'")).toBe(true);
    expect(page.includes("<PluginSettingsPanel section='installed'")).toBe(true);
    expect(page.includes("<PluginSettingsPanel section='market'")).toBe(true);
    expect(mcpMarket.includes("defaultSource='mcpworld'")).toBe(true);

    expect(plugins.includes("section?: 'installed' | 'market' | 'both'")).toBe(true);
    expect(zh.includes('"installedMcpTab": "已安装MCP"')).toBe(true);
    expect(zh.includes('"mcpMarketTab": "MCP市场"')).toBe(true);
    expect(zh.includes('"installedPluginsTab": "已安装插件"')).toBe(true);
    expect(zh.includes('"pluginMarketTab": "插件市场"')).toBe(true);
  });
});
