/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('capability hub navigation', () => {
  test('groups assistants and skills together and keeps MCP in enhanced tools', () => {
    const siderSource = readSource(new URL('./index.tsx', import.meta.url));

    expect(siderSource.includes('SiderAssistantSkillsEntry')).toBe(true);
    expect(siderSource.includes("navTo('/assistants?tab=assistants')")).toBe(true);
    expect(siderSource.includes('SiderMcpEntry')).toBe(true);
    expect(siderSource.includes("navTo('/mcp')")).toBe(true);
    expect(siderSource.includes("pathname.startsWith('/mcp')")).toBe(true);
    expect(siderSource.includes('SiderOpenCapabilitiesEntry')).toBe(true);
    expect(siderSource.includes("navTo('/open-capabilities')")).toBe(true);
    expect(siderSource.includes("pathname.startsWith('/open-capabilities')")).toBe(true);
    expect(siderSource.includes("pathname.startsWith('/open-capabilities') || pathname.startsWith('/mcp')")).toBe(false);

    expect(siderSource.includes('SiderExtensionsEntry')).toBe(false);
    expect(siderSource.includes('SiderAssistantsEntry')).toBe(false);
  });

  test('routes Open Capabilities and preserves MCP legacy destinations', () => {
    const routerSource = readSource(new URL('../Router.tsx', import.meta.url));

    expect(routerSource.includes("path='/open-capabilities'")).toBe(true);
    expect(routerSource.includes("path='/settings/webui' element={<Navigate to='/open-capabilities'")).toBe(true);
    expect(routerSource.includes("path='/settings/tools' element={<Navigate to='/open-capabilities'")).toBe(true);
    expect(routerSource.includes('getHashRouteRedirectUrl')).toBe(true);
    expect(routerSource.includes("return `${origin}/#${pathname}${search}`")).toBe(true);
    expect(routerSource.includes("path='/mcp'")).toBe(true);
    expect(routerSource.includes('LegacyExtensionsRedirect')).toBe(true);
    expect(routerSource.includes("path='/extensions'")).toBe(true);
  });
});
