import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createElement as h, useState } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { SkillInfo } from '@/common/types/skill';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import * as arcoMessageHook from '@/renderer/utils/ui/useArcoMessage';
import { INSTALLED_MARKET_KEY, SKILL_MARKET_CACHE_KEY } from '@/renderer/services/skills/skillMarketProvenance';
import SkillMarketSettings from '../settings/SkillMarketSettings';
import { useSkillsLibrary } from './useSkillsLibrary';

const locale = createInstance();
const restore: Array<() => void> = [];
const marketItem = { id: 'clawhub:owner/ownership', source: 'clawhub', rank: 1, name: 'Owned market skill', description: 'Canonical ownership fixture', url: 'https://clawhub.ai/owner/ownership', install_command: 'openclaw skills install @owner/ownership' };
const skill: SkillInfo = { name: 'installed-canonical', description: 'Canonical instructions', source: 'custom', is_custom: true, location: '/fixture/SKILL.md' };
beforeAll(async () => { await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { common, settings } } } }); });
afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach((dispose) => dispose());
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => { resolve = yes; });
  return { promise, resolve };
}
function OwnedMarket() {
  const library = useSkillsLibrary();
  const [market, setMarket] = useState(1);
  return h('div', null,
    library.messageContext,
    h('output', { 'data-testid': 'owned-library-count' }, library.availableSkills.length),
    h('button', { onClick: () => setMarket(0) }, 'Close market'),
    h('button', { onClick: () => setMarket((value) => value + 1) }, 'Open market'),
    h('button', { onClick: () => void library.refresh() }, 'Refresh library'),
    market > 0 && h(SkillMarketSettings, {
      key: market,
      addedStateLoading: library.loading || Boolean(library.error),
      onInstalled: () => { void library.refresh(); },
    })
  );
}

describe('Skill market library ownership', () => {
  test('keeps installation provenance when a market is reopened and an older library snapshot arrives later', async () => {
    for (const [storage, key, value] of [
      [localStorage, SKILL_MARKET_CACHE_KEY, JSON.stringify({ fetched_at: 1, items: [marketItem], errors: [] })],
      [localStorage, INSTALLED_MARKET_KEY, JSON.stringify({ version: 2, items: {} })],
      [sessionStorage, 'nomifun.skillMarket.autoSynced.v4', '1'],
    ] as const) {
      const previous = storage.getItem(key);
      storage.setItem(key, value);
      restore.push(() => previous === null ? storage.removeItem(key) : storage.setItem(key, previous));
    }
    const staleSnapshot = deferred<SkillInfo[]>();
    const installation = deferred<{ skill_names: string[] }>();
    const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke')
      .mockResolvedValueOnce([])
      .mockImplementationOnce(() => staleSnapshot.promise)
      .mockResolvedValue([skill]);
    const paths = spyOn(ipcBridge.fs.getSkillPaths, 'invoke').mockResolvedValue({ user_skills_dir: '/skills', builtin_skills_dir: '/builtin' });
    const auto = spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]);
    const installing = spyOn(ipcBridge.fs.installSkillMarketItem, 'invoke').mockImplementation(() => installation.promise);
    const message = spyOn(arcoMessageHook, 'useArcoMessage').mockReturnValue([
      { success: () => () => {}, warning: () => () => {}, error: () => () => {} }, null,
    ] as unknown as ReturnType<typeof arcoMessageHook.useArcoMessage>);
    restore.push(...[listing, paths, auto, installing, message].map((mock) => () => mock.mockRestore()));
    const view = render(h(I18nextProvider, { i18n: locale }, h(OwnedMarket)));
    const add = await view.findByRole('button', { name: common.add });
    await waitFor(() => expect((add as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(add);
    expect(installing).toHaveBeenCalledTimes(1);
    fireEvent.click(view.getByRole('button', { name: 'Close market' }));
    fireEvent.click(view.getByRole('button', { name: 'Open market' }));
    expect(listing).toHaveBeenCalledTimes(1);
    fireEvent.click(view.getByRole('button', { name: 'Refresh library' }));
    expect(listing).toHaveBeenCalledTimes(2);

    await act(async () => installation.resolve({ skill_names: [skill.name] }));
    await waitFor(() => expect(view.getByTestId('owned-library-count').textContent).toBe('1'));
    expect(listing).toHaveBeenCalledTimes(3);
    await act(async () => staleSnapshot.resolve([]));
    expect(view.getByTestId('owned-library-count').textContent).toBe('1');
    expect(view.getByRole('button', { name: common.added })).toBeTruthy();
    expect(JSON.parse(localStorage.getItem(INSTALLED_MARKET_KEY) ?? '{}').items[marketItem.id].skill_names).toEqual([skill.name]);
  });
});
