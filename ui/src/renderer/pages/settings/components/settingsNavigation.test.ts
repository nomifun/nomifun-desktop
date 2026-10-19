/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { afterEach, beforeAll, describe, expect, test } from 'bun:test';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { createElement } from 'react';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { generalSettingsHref, resolveGeneralSettingsFocus, resolveGeneralSettingsSection } from './generalSettingsNavigation';
import SettingsSider from './SettingsSider';

const readSource = (url: URL) => readFileSync(url, 'utf8');
const locale = createInstance();

beforeAll(async () => {
  await locale.init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings: zhSettings, ssh: { title: '远程主机' } } } } });
});

afterEach(cleanup);

function LocationProbe() {
  const location = useLocation();
  return createElement('output', { 'data-testid': 'settings-location' }, location.pathname + location.search);
}

function renderSider(path: string) {
  return render(createElement(I18nextProvider, { i18n: locale },
    createElement(MemoryRouter, { initialEntries: [path] },
      createElement(SettingsSider), createElement(LocationProbe))));
}

describe('settings navigation', () => {
  test.each([
    [undefined, 'preferences'], ['preferences', 'preferences'], ['conversation', 'preferences'],
    ['notifications', 'preferences'], ['files', 'data'], ['data', 'data'], ['unknown', 'preferences'],
  ] as const)('resolves the existing section %s into %s', (section, expected) => {
    expect(resolveGeneralSettingsSection(section)).toBe(expected);
  });

  test('preserves valid setting focus and maps existing section links', () => {
    expect(resolveGeneralSettingsFocus('conversation')).toBe('conversation');
    expect(resolveGeneralSettingsFocus('notifications')).toBe('notifications');
    expect(resolveGeneralSettingsFocus('files')).toBe('files');
    expect(resolveGeneralSettingsFocus('data')).toBeUndefined();
    expect(resolveGeneralSettingsFocus('data', 'storage')).toBe('storage');
    expect(resolveGeneralSettingsFocus('preferences', 'reset')).toBe('reset');
    expect(resolveGeneralSettingsFocus('preferences', '[arbitrary-selector]')).toBeUndefined();
    expect(generalSettingsHref('preferences', 'notifications')).toBe('/settings/system?section=preferences&focus=notifications');
  });

  test.each([
    ['preferences', 'preferences'], ['conversation', 'preferences'], ['notifications', 'preferences'],
    ['files', 'data'], ['data', 'data'],
  ])('highlights only the category containing %s', (section, expected) => {
    const view = renderSider(`/settings/system?section=${section}`);
    const selected = view.container.querySelectorAll('.sider-nav-item--active');
    expect(selected.length).toBe(1);
    expect(selected[0].getAttribute('data-settings-id')).toBe(expected);
  });

  test('switches categories while clearing focus and retaining unrelated URL state', () => {
    const view = renderSider('/settings/system?section=conversation&focus=notifications&keep=1');
    fireEvent.click(view.getByRole('button', { name: '数据管理' }));
    expect(view.getByTestId('settings-location').textContent).toBe('/settings/system?section=data&keep=1');
    fireEvent.click(view.getByRole('button', { name: '日常偏好' }));
    expect(view.getByTestId('settings-location').textContent).toBe('/settings/system?section=preferences&keep=1');
  });

  test('does not highlight general settings on the capabilities page', () => {
    const view = renderSider('/settings/permissions?tab=voice-input');
    const selected = view.container.querySelectorAll('.sider-nav-item--active');
    expect(selected.length).toBe(1);
    expect(selected[0].getAttribute('data-settings-id')).toBe('permissions');
    expect(view.queryByRole('button', { name: '通用设置' })).toBeNull();
  });

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
