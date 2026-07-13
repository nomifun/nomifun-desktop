/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('capability hub navigation', () => {
  test('uses compact Remote and Open labels for the Open Capabilities tab', () => {
    const zhSettings = JSON.parse(
      readSource(new URL('../../../services/i18n/locales/zh-CN/settings.json', import.meta.url))
    );
    const enSettings = JSON.parse(
      readSource(new URL('../../../services/i18n/locales/en-US/settings.json', import.meta.url))
    );

    expect(zhSettings.openCapabilities.title).toBe('远程&开放能力');
    expect(zhSettings.openCapabilities.railTitle).toBe('远程&开放能力');
    expect(enSettings.openCapabilities.title).toBe('Remote & Open');
    expect(enSettings.openCapabilities.railTitle).toBe('Remote & Open');
  });

  test('keeps presets, skills, and MCP as independent enhanced-tool destinations', () => {
    const siderSource = readSource(new URL('./index.tsx', import.meta.url));

    expect(siderSource.includes('SiderPresetEntry')).toBe(true);
    expect(siderSource.includes("navTo('/presets')")).toBe(true);
    expect(siderSource.includes("pathname.startsWith('/presets')")).toBe(true);
    expect(siderSource.includes('SiderSkillsEntry')).toBe(true);
    expect(siderSource.includes("navTo('/skills')")).toBe(true);
    expect(siderSource.includes("pathname.startsWith('/skills')")).toBe(true);
    expect(siderSource.includes('SiderMcpEntry')).toBe(true);
    expect(siderSource.includes("navTo('/mcp')")).toBe(true);
    expect(siderSource.includes("pathname.startsWith('/mcp')")).toBe(true);
    expect(siderSource.includes('SiderOpenCapabilitiesEntry')).toBe(true);
    expect(siderSource.includes("navTo('/open-capabilities')")).toBe(true);
    expect(siderSource.includes("pathname.startsWith('/open-capabilities')")).toBe(true);
    expect(siderSource.includes("pathname.startsWith('/open-capabilities') || pathname.startsWith('/mcp')")).toBe(false);

    expect(siderSource.includes('SiderExtensionsEntry')).toBe(false);
  });

  test('routes Open Capabilities and preserves MCP legacy destinations', () => {
    const routerSource = readSource(new URL('../Router.tsx', import.meta.url));

    expect(routerSource.includes("path='/open-capabilities'")).toBe(true);
    expect(routerSource.includes("path='/settings/webui' element={<Navigate to='/open-capabilities'")).toBe(true);
    expect(routerSource.includes("path='/settings/tools' element={<Navigate to='/open-capabilities'")).toBe(true);
    expect(routerSource.includes('getHashRouteRedirectUrl')).toBe(true);
    expect(routerSource.includes("return `${origin}/#${pathname}${search}`")).toBe(true);
    expect(routerSource.includes("path='/mcp'")).toBe(true);
    expect(routerSource.includes("path='/presets'")).toBe(true);
    expect(routerSource.includes("path='/skills'")).toBe(true);
    expect(routerSource.includes('LegacyExtensionsRedirect')).toBe(true);
    expect(routerSource.includes("path='/extensions'")).toBe(true);
  });
});
