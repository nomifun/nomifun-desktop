import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { mcpService } from '@/common/adapter/ipcBridge';
import type { SkillInfo } from '@/common/types/skill';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import { INSTALLED_MARKET_KEY, SKILL_MARKET_CACHE_KEY } from '@/renderer/services/skills/skillMarketProvenance';
import EnhancedToolsPage from './index';

const locale = createInstance();
const restore: Array<() => void> = [];
const track = <T extends { mockRestore: () => void }>(spy: T): T => {
  restore.push(() => spy.mockRestore());
  return spy;
};
const marketItem = {
  id: 'clawhub:owner/navigation-skill', source: 'clawhub', rank: 1,
  name: 'Navigation Skill', description: 'Imported through the real market.',
  url: 'https://clawhub.ai/owner/navigation-skill',
  install_command: 'openclaw skills install @owner/navigation-skill',
};
const skill = (name: string): SkillInfo => ({
  name, description: `${name} instructions`, location: `/fixture/skills/${name}/SKILL.md`,
  source: 'custom', is_custom: true,
});
const importedSkill = skill('navigation-imported');

beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } } });
});
afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach((dispose) => dispose());
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => { resolve = yes; });
  return { promise, resolve };
}

function fixture() {
  // Keep the actual unified page, panels, markets and catalog hooks mounted.
  // Only backend boundaries, theme and this fixture's browser cache are isolated.
  for (const [storage, key, value] of [
    [localStorage, SKILL_MARKET_CACHE_KEY, JSON.stringify({ fetched_at: 1, items: [marketItem], errors: [] })],
    [localStorage, INSTALLED_MARKET_KEY, JSON.stringify({ version: 2, items: {} })],
    [localStorage, 'nomifun.skillMarket.installed.v1', '{}'],
    [sessionStorage, 'nomifun.skillMarket.autoSynced.v4', '1'],
    [localStorage, 'nomifun.mcpMarket.rankings.v1', JSON.stringify({ fetched_at: 1, items: [], errors: [] })],
    [sessionStorage, 'nomifun.mcpMarket.autoSynced.v1', '1'],
  ] as const) {
    const previous = storage.getItem(key);
    storage.setItem(key, value);
    restore.push(() => previous === null ? storage.removeItem(key) : storage.setItem(key, previous));
  }
  let skills = [skill('alpha'), skill('beta')];
  const installation = deferred<{ skill_names: string[] }>();
  const listing = track(spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockImplementation(async () => structuredClone(skills)));
  track(spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]));
  track(spyOn(ipcBridge.fs.getSkillPaths, 'invoke').mockResolvedValue({
    user_skills_dir: '/fixture/skills', builtin_skills_dir: '/fixture/builtin',
  }));
  track(spyOn(mcpService.listServers, 'invoke').mockResolvedValue([]));
  const installing = track(spyOn(ipcBridge.fs.installSkillMarketItem, 'invoke').mockImplementation(() => installation.promise));
  track(spyOn(themeContext, 'useThemeContext').mockReturnValue({
    theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {},
  }));
  track(spyOn(Message, 'error').mockReturnValue(() => {}));
  track(spyOn(Message, 'warning').mockReturnValue(() => {}));
  track(spyOn(Message, 'success').mockReturnValue(() => {}));

  const mount = async (path = '/enhanced-tools?category=mcp&keep=1') => {
    const router = createMemoryRouter([
      { path: '/enhanced-tools', element: <EnhancedToolsPage /> },
      { path: '/elsewhere', element: <div>Other destination</div> },
    ], { initialEntries: [path] });
    restore.push(() => router.dispose());
    let view!: ReturnType<typeof render>;
    await act(async () => {
      view = render(<I18nextProvider i18n={locale}><RouterProvider router={router} /></I18nextProvider>);
    });
    await waitFor(() => expect(listing).toHaveBeenCalled());
    return { ...view, router };
  };
  const completeInstallation = async () => {
    skills = [...skills, importedSkill];
    await act(async () => { installation.resolve({ skill_names: [importedSkill.name] }); });
  };
  return { mount, listing, installing, completeInstallation };
}

type PageFixture = Awaited<ReturnType<ReturnType<typeof fixture>['mount']>>;

async function openMarket(view: PageFixture) {
  fireEvent.click(view.getByTestId('enhanced-tools-add'));
  fireEvent.click(await view.findByTestId('enhanced-tools-open-market'));
  await waitFor(() => expect(new URLSearchParams(view.router.state.location.search).get('view')).toBe('market'));
}

async function beginSkillInstall(view: PageFixture) {
  const add = await view.findByTestId('btn-add-market-skill-clawhub-owner-navigation-skill');
  await waitFor(() => expect((add as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(add);
}

describe('Enhanced Tools navigation with the real page', () => {
  test('category changes and browser back restore independent searches and keep query state', async () => {
    const f = fixture();
    const view = await f.mount();
    fireEvent.change(view.getByTestId('input-search-mcp'), { target: { value: 'server search' } });
    fireEvent.click(view.getByRole('tab', { name: /^Skills/ }));
    await view.findByTestId('input-search-skills');
    fireEvent.change(view.getByTestId('input-search-skills'), { target: { value: 'alpha' } });
    expect(new URLSearchParams(view.router.state.location.search).get('category')).toBe('skills');
    expect(new URLSearchParams(view.router.state.location.search).get('keep')).toBe('1');
    expect(view.queryByTestId('skill-card-beta')).toBeNull();

    await act(async () => { await view.router.navigate(-1); });
    expect((view.getByTestId('input-search-mcp') as HTMLInputElement).value).toBe('server search');
    expect(view.getByRole('tab', { name: /^MCP/ }).getAttribute('aria-selected')).toBe('true');
    await act(async () => { await view.router.navigate(1); });
    expect((view.getByTestId('input-search-skills') as HTMLInputElement).value).toBe('alpha');
    expect(new URLSearchParams(view.router.state.location.search).get('keep')).toBe('1');
  });

  test('market opening, closing and browser history preserve category and unrelated parameters', async () => {
    const f = fixture();
    const view = await f.mount('/enhanced-tools?category=skills&keep=1');
    await openMarket(view);
    expect(new URLSearchParams(view.router.state.location.search).get('category')).toBe('skills');
    expect(new URLSearchParams(view.router.state.location.search).get('keep')).toBe('1');
    fireEvent.click(view.getByRole('button', { name: settings.enhancedTools.backToList }));
    await waitFor(() => expect(new URLSearchParams(view.router.state.location.search).has('view')).toBe(false));
    await act(async () => { await view.router.navigate(-1); });
    expect(new URLSearchParams(view.router.state.location.search).get('view')).toBe('market');
    await view.findByTestId('skill-market-surface');
    expect(new URLSearchParams(view.router.state.location.search).get('keep')).toBe('1');
  });

  test('installing a market skill returns to its refreshed library and highlights the imported item', async () => {
    const f = fixture();
    const view = await f.mount('/enhanced-tools?category=skills&keep=1');
    fireEvent.change(view.getByTestId('input-search-skills'), { target: { value: 'alpha' } });
    await openMarket(view);
    await beginSkillInstall(view);
    expect(f.installing).toHaveBeenCalledTimes(1);
    await f.completeInstallation();
    const card = await view.findByTestId('skill-card-navigation-imported');
    await waitFor(() => expect(card.classList.contains('enhanced-tools-skill-card--highlighted')).toBe(true));
    const params = new URLSearchParams(view.router.state.location.search);
    expect(params.get('category')).toBe('skills');
    expect(params.has('view')).toBe(false);
    expect(params.get('keep')).toBe('1');
    expect((view.getByTestId('input-search-skills') as HTMLInputElement).value).toBe('');
    expect(view.getByRole('tab', { name: /^Skills/ }).textContent).toContain('3');
  });

  test('a pending installation cannot take over another category after browser navigation', async () => {
    const f = fixture();
    const view = await f.mount('/enhanced-tools?category=skills&view=market&keep=1');
    await beginSkillInstall(view);
    await act(async () => { await view.router.navigate('/enhanced-tools?category=mcp&keep=2'); });
    await f.completeInstallation();
    expect(view.router.state.location.pathname + view.router.state.location.search).toBe('/enhanced-tools?category=mcp&keep=2');
    expect(view.getByRole('tab', { name: /^MCP/ }).getAttribute('aria-selected')).toBe('true');
  });

  test('a pending installation cannot bring back a page the user has left', async () => {
    const f = fixture();
    const view = await f.mount('/enhanced-tools?category=skills&view=market&keep=1');
    await beginSkillInstall(view);
    await act(async () => { await view.router.navigate('/elsewhere?keep=outside'); });
    const readsBeforeCompletion = f.listing.mock.calls.length;
    await f.completeInstallation();
    expect(view.router.state.location.pathname + view.router.state.location.search).toBe('/elsewhere?keep=outside');
    expect(view.getByText('Other destination')).toBeTruthy();
    expect(f.listing.mock.calls.length).toBe(readsBeforeCompletion);
    expect(JSON.parse(localStorage.getItem(INSTALLED_MARKET_KEY) ?? '{}').items[marketItem.id].skill_names).toEqual([importedSkill.name]);
  });

  test('an old installation does not close a market drawer opened afterward', async () => {
    const f = fixture();
    const view = await f.mount('/enhanced-tools?category=skills&view=market&keep=1');
    await beginSkillInstall(view);
    fireEvent.click(view.getByRole('button', { name: settings.enhancedTools.backToList }));
    await waitFor(() => expect(new URLSearchParams(view.router.state.location.search).has('view')).toBe(false));
    await openMarket(view);
    await view.findByTestId('skill-market-surface');
    await f.completeInstallation();
    const params = new URLSearchParams(view.router.state.location.search);
    expect(params.get('category')).toBe('skills');
    expect(params.get('view')).toBe('market');
    expect(params.get('keep')).toBe('1');
    expect(view.getByTestId('skill-market-surface')).toBeTruthy();
    await waitFor(() => expect(view.getByTestId('btn-add-market-skill-clawhub-owner-navigation-skill').textContent).toContain(common.added));
  });
});
