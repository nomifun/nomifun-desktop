/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, render } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { configService } from '@/common/config/configService';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '@renderer/pages/settings/DisplaySettings/presets';
import { processCustomCss } from '@renderer/utils/theme/customCssProcessor';
import { THEME_CONTROL_CONTRACT_STYLE_ID } from '@renderer/utils/theme/themeControlContract';
import AppThemeRuntime from './AppThemeRuntime';

const CUSTOM_CSS_STYLE_ID = 'user-defined-custom-css';
const classic = PRESET_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)!;
const frosted = PRESET_THEMES.find((theme) => theme.id === 'frosted-glass')!;
const restore: Array<() => void> = [];

afterEach(() => {
  cleanup();
  for (const reset of restore.splice(0).reverse()) reset();
  configService.reset();
  document.getElementById(CUSTOM_CSS_STYLE_ID)?.remove();
  document.getElementById(THEME_CONTROL_CONTRACT_STYLE_ID)?.remove();
});

function fixture() {
  configService.reset();
  const requests: Array<{
    method: string;
    path: string;
    body: unknown;
    reply: (body?: unknown) => void;
  }> = [];
  const fetching = spyOn(globalThis, 'fetch').mockImplementation((input, init) => new Promise<Response>((resolve) => {
    requests.push({
      method: init?.method ?? 'GET',
      path: String(input),
      body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
      reply: (body) => resolve(body === undefined
        ? new Response(null, { status: 204 })
        : new Response(JSON.stringify(body), { headers: { 'Content-Type': 'application/json' } })),
    });
  }));
  restore.push(() => fetching.mockRestore());
  const events: string[] = [];
  const onCssUpdated = (event: Event) => events.push((event as CustomEvent<{ customCss: string }>).detail.customCss);
  window.addEventListener('custom-css-updated', onCssUpdated);
  restore.push(() => window.removeEventListener('custom-css-updated', onCssUpdated));
  const mount = () => render(<MemoryRouter><AppThemeRuntime /></MemoryRouter>);
  const answer = async (index: number, body?: unknown) => {
    await act(async () => { requests[index]!.reply(body); });
  };
  return { requests, events, mount, answer };
}

test('waits for persisted settings before applying or writing a default theme', async () => {
  const f = fixture();
  f.mount();
  await act(async () => {});

  expect(f.requests.map(({ method }) => method)).toEqual(['GET']);
  expect(f.requests[0]!.path).toEndWith('/api/settings/client');
  expect(configService.get('css.activeThemeId')).toBeUndefined();
  expect(f.events).toEqual([]);
  expect(document.getElementById(CUSTOM_CSS_STYLE_ID)).toBeNull();

  await f.answer(0, { 'css.activeThemeId': frosted.id, customCss: frosted.css, 'css.themes': [] });

  expect(configService.get('css.activeThemeId')).toBe(frosted.id);
  expect(configService.get('customCss')).toBe(frosted.css);
  expect(f.requests.map(({ method }) => method)).toEqual(['GET']);
  expect(f.events).toEqual([frosted.css]);
  expect(document.getElementById(CUSTOM_CSS_STYLE_ID)?.textContent).toBe(processCustomCss(frosted.css));
});

test('a delayed retired-theme repair cannot broadcast over a newer user selection', async () => {
  const f = fixture();
  f.mount();
  await act(async () => {});
  await f.answer(0, { 'css.activeThemeId': 'rhythm-dark', customCss: 'body { color: hotpink; }', 'css.themes': [] });

  expect(f.requests.map(({ method }) => method)).toEqual(['GET', 'PUT']);
  expect(f.requests[1]!.body).toEqual({ 'css.activeThemeId': classic.id, customCss: classic.css });

  let selection!: Promise<void>;
  act(() => { selection = configService.setBatch({ 'css.activeThemeId': frosted.id, customCss: frosted.css }); });
  await act(async () => {
    f.requests[2]!.reply();
    await selection;
    window.dispatchEvent(new CustomEvent('custom-css-updated', { detail: { customCss: frosted.css } }));
  });
  expect(document.getElementById(CUSTOM_CSS_STYLE_ID)?.textContent).toBe(processCustomCss(frosted.css));

  f.events.length = 0;
  await f.answer(1);

  expect(f.events).toEqual([]);
  expect(configService.get('css.activeThemeId')).toBe(frosted.id);
  expect(configService.get('customCss')).toBe(frosted.css);
  expect(document.getElementById(CUSTOM_CSS_STYLE_ID)?.textContent).toBe(processCustomCss(frosted.css));
  expect(f.requests.map(({ method }) => method)).toEqual(['GET', 'PUT', 'PUT']);
});

test('renders the Classic fallback offline without persisting repair settings', async () => {
  configService.reset();
  const methods: string[] = [];
  const fetching = spyOn(globalThis, 'fetch').mockImplementation(async (_input, init) => {
    methods.push(init?.method ?? 'GET');
    throw new Error('test settings unavailable');
  });
  const warning = spyOn(console, 'warn').mockImplementation(() => {});
  restore.push(() => fetching.mockRestore(), () => warning.mockRestore());

  render(<MemoryRouter><AppThemeRuntime /></MemoryRouter>);
  await act(async () => {});

  expect(methods.length).toBeGreaterThan(0);
  expect(methods.every((method) => method === 'GET')).toBe(true);
  expect(configService.isInitialized()).toBe(false);
  expect(configService.get('css.activeThemeId')).toBeUndefined();
  expect(configService.get('customCss')).toBeUndefined();
  expect(document.getElementById(CUSTOM_CSS_STYLE_ID)?.textContent).toBe(processCustomCss(classic.css));
});
