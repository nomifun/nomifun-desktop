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
    const pageWrapperSource = readSource(new URL('./SettingsPageWrapper.tsx', import.meta.url));

    for (const id of ['system', 'execution-engines', 'browser-use', 'computer-use', 'about']) {
      expect(siderSource.includes(`'${id}'`)).toBe(true);
      expect(pageWrapperSource.includes(`id: '${id}'`)).toBe(true);
    }

    expect(siderSource.indexOf("'system'")).toBeLessThan(siderSource.indexOf("'execution-engines'"));
    expect(siderSource.indexOf("'execution-engines'")).toBeLessThan(siderSource.indexOf("'browser-use'"));
    expect(siderSource.indexOf("'browser-use'")).toBeLessThan(siderSource.indexOf("'computer-use'"));
    expect(siderSource.indexOf("'computer-use'")).toBeLessThan(siderSource.indexOf("'about'"));
  });

  test('routes execution engines directly and keeps legacy links compatible', () => {
    const routerSource = readSource(new URL('../../../components/layout/Router.tsx', import.meta.url));
    const engineTabsSource = readSource(
      new URL('../../../components/settings/SettingsModal/contents/AgentModalContent.tsx', import.meta.url)
    );

    for (const path of ['/settings/execution-engines', '/settings/browser-use', '/settings/computer-use']) {
      expect(routerSource.includes(`path='${path}'`)).toBe(true);
    }

    expect(routerSource.includes("import('@renderer/pages/settings/AgentSettings')")).toBe(true);
    expect(routerSource.includes("to='/settings/execution-engines?tab=runtime'")).toBe(true);
    expect(routerSource.includes("to='/models?section=agents'")).toBe(false);
    expect(engineTabsSource.includes("key='runtime'")).toBe(true);
    expect(engineTabsSource.includes('<AgentRuntimeSettingsContent />')).toBe(true);
    expect(routerSource.includes("path='/settings/browser-use' element={<Navigate to='/settings/system'")).toBe(false);
    expect(routerSource.includes("path='/settings/computer-use' element={<Navigate to='/settings/system'")).toBe(false);
  });
});
