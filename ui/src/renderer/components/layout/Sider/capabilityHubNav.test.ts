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

  test('exposes Agent as the public authoring destination and keeps Skills/MCP separate', () => {
    const siderSource = readSource(new URL('./index.tsx', import.meta.url));

    expect(siderSource.includes('SiderAgentEntry')).toBe(true);
    expect(siderSource.includes("navTo('/agent')")).toBe(true);
    expect(
      siderSource.includes(
        "navTo('/guid', false, { resetAgentSelection: true })"
      )
    ).toBe(true);
    expect(siderSource.includes("pathname === '/agent' || pathname.startsWith('/agent-sessions/')")).toBe(true);
    expect(siderSource.includes('SiderPresetEntry')).toBe(false);
    expect(siderSource.includes("navTo('/presets')")).toBe(false);
    expect(siderSource.includes("pathname.startsWith('/presets')")).toBe(false);
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

  test('routes Open Capabilities and preserves only supported product destinations', () => {
    const routerSource = readSource(new URL('../Router.tsx', import.meta.url));

    expect(routerSource.includes("path='/open-capabilities'")).toBe(true);
    expect(routerSource.includes("path='/settings/webui' element={<Navigate to='/open-capabilities'")).toBe(true);
    expect(routerSource.includes("path='/settings/tools' element={<Navigate to='/open-capabilities'")).toBe(true);
    expect(routerSource.includes('getHashRouteRedirectUrl')).toBe(true);
    expect(routerSource.includes("return `${origin}/#${pathname}${search}`")).toBe(true);
    expect(routerSource.includes("path='/mcp'")).toBe(true);
    expect(routerSource.includes("path='/agent'")).toBe(true);
    expect(routerSource.includes('LegacyAgentAuthoringRedirect')).toBe(false);
    expect(routerSource.includes("path='/presets'")).toBe(false);
    expect(routerSource.includes("path='/settings/agent-presets/*'")).toBe(false);
    expect(routerSource.includes("path='/settings/agent'")).toBe(false);
    expect(routerSource.includes("path='/skills'")).toBe(true);
    expect(routerSource.includes('LegacyExtensionsRedirect')).toBe(false);
    expect(routerSource.includes("path='/extensions'")).toBe(false);
  });

  test('keeps unified Browser settings reachable when Browser Use is disabled', () => {
    const siderSource = readSource(new URL('./index.tsx', import.meta.url));
    const routerSource = readSource(new URL('../Router.tsx', import.meta.url));

    expect(siderSource.includes('SiderBrowserEntry')).toBe(true);
    expect(siderSource.includes('isDesktopShell() || browserOverview?.supported !== false')).toBe(true);
    expect(siderSource.includes('browserOverview?.enabled !== false')).toBe(false);
    expect(
      routerSource.includes(
        "path='/settings/browser-use' element={<Navigate to='/browser?tab=settings' replace />}"
      )
    ).toBe(true);
  });
});
