import { afterEach, beforeAll, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { createElement, type PropsWithChildren } from 'react';
import { I18nextProvider } from 'react-i18next';
import { configService } from '@/common/config/configService';
import type { ConfigKeyMap } from '@/common/config/configKeys';
import type { ICssTheme } from '@/common/config/storage';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '@/renderer/pages/settings/DisplaySettings/presets';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import { useCssTheme } from './useCssTheme';

type ThemeConfig = Partial<Pick<ConfigKeyMap, 'customCss' | 'css.activeThemeId' | 'css.themes'>>;
const classic = PRESET_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)!;
const frosted = PRESET_THEMES.find((theme) => theme.id === 'frosted-glass')!;
const locale = createInstance();
const restores: Array<() => void> = [];
const wrapper = ({ children }: PropsWithChildren) => createElement(I18nextProvider, { i18n: locale }, children);
const userTheme = (overrides: Partial<ICssTheme> = {}): ICssTheme => ({
  id: 'custom-user-theme', name: 'My theme', css: ':root { --primary: #123456; }', is_preset: false, ...overrides,
});

beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings } } } });
});
beforeEach(() => configService.reset());
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach((restore) => restore());
  configService.reset();
});

function fixture(
  initial: ThemeConfig = { customCss: classic.css, 'css.activeThemeId': classic.id },
  persist?: () => Promise<void>,
  load?: () => Promise<ThemeConfig>
) {
  const writes: ThemeConfig[] = [];
  let reads = 0;
  const request = spyOn(globalThis, 'fetch').mockImplementation(async (url, options) => {
    expect(String(url)).toEndWith('/api/settings/client');
    if (options?.method === 'GET') {
      reads += 1;
      return Response.json(load ? await load() : initial);
    }
    expect(options?.method).toBe('PUT');
    writes.push(JSON.parse(String(options?.body)) as ThemeConfig);
    await persist?.();
    return new Response(null, { status: 204 });
  });
  const success = spyOn(Message, 'success').mockReturnValue(() => {});
  const error = spyOn(Message, 'error').mockReturnValue(() => {});
  restores.push(() => request.mockRestore(), () => success.mockRestore(), () => error.mockRestore());

  const cssEvents: string[] = [];
  const onCssUpdated = (event: Event) => cssEvents.push((event as CustomEvent<{ customCss: string }>).detail.customCss);
  window.addEventListener('custom-css-updated', onCssUpdated);
  restores.push(() => window.removeEventListener('custom-css-updated', onCssUpdated));
  const mount = async (initialize = true) => {
    if (initialize) await configService.initialize();
    const hook = renderHook(() => useCssTheme(), { wrapper });
    await act(async () => {});
    return hook;
  };
  return { mount, writes, cssEvents, success, error, get reads() { return reads; } };
}

describe('useCssTheme with the real config service', () => {
  test('waits for initial settings before resolving the selected theme', async () => {
    let completeLoad!: (config: ThemeConfig) => void;
    const loading = new Promise<ThemeConfig>((resolve) => { completeLoad = resolve; });
    const f = fixture({}, undefined, () => loading);
    const hook = await f.mount(false);
    expect(f.reads).toBe(1);
    expect(configService.isInitialized()).toBe(false);
    expect(f.writes).toEqual([]);

    await act(async () => { completeLoad({ customCss: frosted.css, 'css.activeThemeId': frosted.id }); });
    expect(configService.isInitialized()).toBe(true);
    expect(configService.get('customCss')).toBe(frosted.css);
    expect(hook.result.current.activeThemeId).toBe(frosted.id);
    expect(f.writes).toEqual([]);
    expect(f.cssEvents).toEqual([]);
  });

  test('displays Classic without persisting defaults when settings are unavailable', async () => {
    const warning = spyOn(console, 'warn').mockImplementation(() => {});
    restores.push(() => warning.mockRestore());
    const f = fixture({}, undefined, async () => { throw new Error('test offline'); });
    const hook = await f.mount(false);
    expect(f.reads).toBe(1);
    expect(configService.isInitialized()).toBe(false);
    expect(hook.result.current.activeThemeId).toBe(classic.id);
    expect(hook.result.current.themes).toEqual(PRESET_THEMES);
    expect(f.writes).toEqual([]);
  });

  test('starts with Classic selected and persists the current built-in catalog default', async () => {
    const f = fixture({});
    const hook = await f.mount();
    expect(hook.result.current.activeThemeId).toBe(DEFAULT_THEME_ID);
    expect(hook.result.current.themes).toEqual(PRESET_THEMES);
    expect(f.writes).toEqual([{ customCss: classic.css, 'css.activeThemeId': classic.id }]);
    expect(f.cssEvents).toEqual([classic.css]);
  });

  test('replaces retired selection and stored preset snapshots in one settings write', async () => {
    const custom = userTheme();
    const retired = { id: 'rhythm-dark', name: 'Retired preset', css: 'body { color: red; }', is_preset: true };
    const f = fixture({
      customCss: retired.css,
      'css.activeThemeId': retired.id,
      'css.themes': [retired, { ...classic, css: 'obsolete classic CSS' }, frosted, custom],
    });
    const hook = await f.mount();
    expect(f.writes).toEqual([{
      customCss: classic.css, 'css.activeThemeId': classic.id, 'css.themes': [custom],
    }]);
    expect(configService.get('css.themes')).toEqual([custom]);
    expect(hook.result.current.themes).toEqual([...PRESET_THEMES, custom]);
    expect(hook.result.current.activeThemeId).toBe(classic.id);
  });

  test('applies Frosted Glass with its selection and CSS in one write before notifying', async () => {
    let completeWrite!: () => void;
    const persisted = new Promise<void>((resolve) => { completeWrite = resolve; });
    const f = fixture(undefined, () => persisted);
    const hook = await f.mount();
    let pending!: Promise<void>;
    await act(async () => { pending = hook.result.current.selectTheme(frosted); });
    expect(f.writes).toEqual([{ customCss: frosted.css, 'css.activeThemeId': frosted.id }]);
    expect(f.cssEvents).toEqual([]);
    expect(f.success).not.toHaveBeenCalled();
    expect(hook.result.current.activeThemeId).toBe(classic.id);
    await act(async () => { completeWrite(); await pending; });
    expect(configService.get('customCss')).toBe(frosted.css);
    expect(hook.result.current.activeThemeId).toBe(frosted.id);
    expect(f.cssEvents).toEqual([frosted.css]);
    expect(f.success).toHaveBeenCalledWith('Applied theme: Frosted Glass');
    expect(f.error).not.toHaveBeenCalled();
  });

  test('a delayed retired-theme reconciliation cannot restore Classic after selecting Frosted Glass', async () => {
    let completeReconciliation!: () => void;
    const reconciliation = new Promise<void>((resolve) => { completeReconciliation = resolve; });
    let writeCount = 0;
    const f = fixture(
      { customCss: 'retired CSS', 'css.activeThemeId': 'rhythm-dark' },
      () => ++writeCount === 1 ? reconciliation : Promise.resolve()
    );
    const hook = await f.mount();
    expect(f.writes).toEqual([{ customCss: classic.css, 'css.activeThemeId': classic.id }]);
    await act(async () => { await hook.result.current.selectTheme(frosted); });
    expect(hook.result.current.activeThemeId).toBe(frosted.id);
    expect(f.cssEvents).toEqual([frosted.css]);

    await act(async () => { completeReconciliation(); });
    expect(configService.get('css.activeThemeId')).toBe(frosted.id);
    expect(configService.get('customCss')).toBe(frosted.css);
    expect(hook.result.current.activeThemeId).toBe(frosted.id);
    expect(f.cssEvents).toEqual([frosted.css]);
    expect(f.writes).toEqual([
      { customCss: classic.css, 'css.activeThemeId': classic.id },
      { customCss: frosted.css, 'css.activeThemeId': frosted.id },
    ]);
  });

  test('keeps mounted settings and footer hooks synchronized after saving and selecting', async () => {
    const f = fixture();
    const settingsHook = await f.mount();
    const footerHook = await f.mount();
    const data = { name: 'Shared user theme', css: ':root { --primary: #654321; }' };
    await act(async () => { await settingsHook.result.current.saveUserTheme(data, null); });
    const saved = configService.get('css.themes')![0]!;
    expect(settingsHook.result.current.themes).toEqual([...PRESET_THEMES, saved]);
    expect(footerHook.result.current.themes).toEqual(settingsHook.result.current.themes);
    await act(async () => { await footerHook.result.current.selectTheme(saved); });
    expect(settingsHook.result.current.activeThemeId).toBe(saved.id);
    expect(footerHook.result.current.activeThemeId).toBe(saved.id);
    expect(f.writes).toEqual([
      { 'css.themes': [saved] },
      { customCss: data.css, 'css.activeThemeId': saved.id },
    ]);
  });

  test('deleting the active custom theme atomically applies Classic without clearing CSS', async () => {
    const active = userTheme();
    const retained = userTheme({ id: 'retained-user-theme', name: 'Retained theme' });
    const f = fixture({ customCss: active.css, 'css.activeThemeId': active.id, 'css.themes': [active, retained] });
    const hook = await f.mount();
    const statesAtCssChange: ThemeConfig[] = [];
    restores.push(configService.subscribe('customCss', (css) => statesAtCssChange.push({
      customCss: css as string,
      'css.activeThemeId': configService.get('css.activeThemeId'),
      'css.themes': configService.get('css.themes'),
    })));
    await act(async () => { await hook.result.current.deleteUserTheme(active.id); });
    const expected = { customCss: classic.css, 'css.activeThemeId': classic.id, 'css.themes': [retained] };
    expect(f.writes).toEqual([expected]);
    expect(statesAtCssChange).toEqual([expected]);
    expect(f.cssEvents).toEqual([classic.css]);
    expect(hook.result.current.activeThemeId).toBe(classic.id);
    expect(hook.result.current.themes).toEqual([...PRESET_THEMES, retained]);
  });

  test('saves and edits custom themes without retired timestamp metadata', async () => {
    const f = fixture();
    const hook = await f.mount();
    await act(async () => { await hook.result.current.saveUserTheme({ name: 'New theme', css: 'body { color: navy; }' }, null); });
    const saved = configService.get('css.themes')![0]!;
    expect(saved.id).toBeTruthy();
    expect(saved).toEqual({ id: saved.id, name: 'New theme', css: 'body { color: navy; }', is_preset: false });
    await act(async () => { await hook.result.current.saveUserTheme({ name: 'Edited theme', css: 'body { color: green; }' }, saved); });
    const edited = { id: saved.id, name: 'Edited theme', css: 'body { color: green; }', is_preset: false };
    expect(f.writes).toEqual([{ 'css.themes': [saved] }, { 'css.themes': [edited] }]);
    expect(hook.result.current.themes).toEqual([...PRESET_THEMES, edited]);
  });
});
