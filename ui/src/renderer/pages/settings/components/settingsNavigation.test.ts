/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

describe('settings navigation', () => {
  test('exposes execution engines as a first-level settings page', () => {
    const siderSource = readSource(new URL('./SettingsSider.tsx', import.meta.url));
    for (const id of ['system', 'execution-engines', 'javascript-runtime', 'computer-use', 'about']) {
      expect(siderSource.includes(`'${id}'`)).toBe(true);
    }

    expect(siderSource.includes("'browser-use'")).toBe(false);
    expect(siderSource.indexOf("'system'")).toBeLessThan(siderSource.indexOf("'execution-engines'"));
    expect(siderSource.indexOf("'execution-engines'")).toBeLessThan(siderSource.indexOf("'javascript-runtime'"));
    expect(siderSource.indexOf("'javascript-runtime'")).toBeLessThan(siderSource.indexOf("'computer-use'"));
    expect(siderSource.indexOf("'computer-use'")).toBeLessThan(siderSource.indexOf("'about'"));
  });

  test('routes execution engines directly without an Agent authoring entry', () => {
    const routerSource = readSource(new URL('../../../components/layout/Router.tsx', import.meta.url));
    const enginePageSource = readSource(new URL('../ExecutionEngines/index.tsx', import.meta.url));
    const javascriptPageSource = readSource(new URL('../JavaScriptRuntimeSettings.tsx', import.meta.url));

    for (const path of ['/settings/execution-engines', '/settings/javascript-runtime', '/settings/computer-use']) {
      expect(routerSource.includes(`path='${path}'`)).toBe(true);
    }

    expect(routerSource.includes("import('@renderer/pages/settings/ExecutionEngines')")).toBe(true);
    expect(routerSource.includes("import('@renderer/pages/settings/JavaScriptRuntimeSettings')")).toBe(true);
    expect(routerSource.includes("path='/agent'")).toBe(true);
    expect(routerSource.includes('LegacyAgentAuthoringRedirect')).toBe(false);
    expect(routerSource.includes("path='/settings/agent'")).toBe(false);
    expect(routerSource.includes("path='/settings/agent-presets/*'")).toBe(false);
    expect(routerSource.includes("to='/settings/execution-engines'")).toBe(true);
    expect(routerSource.includes("to='/models?section=agents'")).toBe(false);
    expect(enginePageSource.includes('AgentModalContent')).toBe(false);
    expect(enginePageSource.includes('<RuntimeManager />')).toBe(false);
    expect(enginePageSource.includes('agentPlatform.runtimeEngines.list.invoke')).toBe(true);
    expect(javascriptPageSource.includes('<RuntimeManager />')).toBe(true);
    expect(routerSource.includes("path='/settings/browser-use'")).toBe(false);
    expect(routerSource.includes("path='/browser'")).toBe(false);
    expect(routerSource.includes("path='/settings/computer-use' element={<Navigate to='/settings/system'")).toBe(false);
  });
});
