/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('settings navigation', () => {
  test('exposes Nomi Runtime diagnostics without a selectable Plugin runtime destination', () => {
    const siderSource = readSource(new URL('./SettingsSider.tsx', import.meta.url));
    for (const id of ['system', 'permissions', 'execution-engines', 'about']) {
      expect(siderSource.includes(`'${id}'`)).toBe(true);
    }

    expect(siderSource.includes("'browser-use'")).toBe(false);
    expect(siderSource.indexOf("'system'")).toBeLessThan(siderSource.indexOf("'permissions'"));
    expect(siderSource.indexOf("'permissions'")).toBeLessThan(siderSource.indexOf("'execution-engines'"));
    expect(siderSource.indexOf("'execution-engines'")).toBeLessThan(siderSource.indexOf("'about'"));
    expect(siderSource.includes("'javascript-runtime'")).toBe(false);
  });

  test('routes one Nomi Runtime diagnostics page without an Agent Runtime selector', () => {
    const routerSource = readSource(new URL('../../../components/layout/Router.tsx', import.meta.url));
    const enginePageSource = readSource(new URL('../ExecutionEngines/index.tsx', import.meta.url));
    const modelHubSource = readSource(new URL('../../modelHub/index.tsx', import.meta.url));

    for (const path of ['/settings/execution-engines', '/settings/permissions']) {
      expect(routerSource.includes(`path='${path}'`)).toBe(true);
    }

    expect(routerSource.includes("import('@renderer/pages/settings/ExecutionEngines')")).toBe(true);
    expect(routerSource.includes("/settings/javascript-runtime")).toBe(false);
    expect(routerSource.includes("path='/agent'")).toBe(true);
    expect(routerSource.includes('LegacyAgentAuthoringRedirect')).toBe(false);
    expect(routerSource.includes("path='/settings/agent'")).toBe(false);
    expect(routerSource.includes("path='/settings/agent-presets/*'")).toBe(false);
    expect(modelHubSource.includes("to='/settings/execution-engines'")).toBe(true);
    expect(routerSource.includes("to='/models?section=agents'")).toBe(false);
    expect(enginePageSource.includes('AgentModalContent')).toBe(false);
    expect(enginePageSource.includes('<RuntimeManager />')).toBe(false);
    expect(enginePageSource.includes('agentPlatform.runtime.get.invoke')).toBe(true);
    expect(enginePageSource.includes('nomifun.coding')).toBe(false);
    expect(enginePageSource.includes('<Select')).toBe(false);
    expect(routerSource.includes("path='/settings/browser-use'")).toBe(false);
    expect(routerSource.includes("path='/settings/voice-input' element={<Navigate to='/settings/permissions?tab=voice-input'")).toBe(true);
    expect(routerSource.includes("path='/browser'")).toBe(false);
    expect(routerSource.includes("path='/settings/computer-use' element={<Navigate to='/settings/permissions?tab=computer-use'")).toBe(true);
  });
});
