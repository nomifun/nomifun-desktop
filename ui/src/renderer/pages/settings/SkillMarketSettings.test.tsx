import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createElement as h } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import * as arcoMessageHook from '@/renderer/utils/ui/useArcoMessage';
import SkillMarketSettings from './SkillMarketSettings';

const CACHE_KEY = 'nomifun.skillMarket.rankings.v4';
const AUTO_SYNC_KEY = 'nomifun.skillMarket.autoSynced.v4';
const INSTALLED_MARKET_KEY = 'nomifun.skillMarket.installed.v1';
const marketItem = {
  id: 'clawhub:owner/demo',
  source: 'clawhub' as const,
  rank: 1,
  name: 'Demo Skill',
  description: 'A fixture skill.',
  url: 'https://clawhub.ai/owner/skills/demo',
  install_command: 'openclaw skills install @owner/demo',
};

const locale = createInstance();
const restore: Array<() => void> = [];

beforeAll(async () => {
  await locale.init({
    lng: 'en-US',
    resources: { 'en-US': { translation: { common, settings } } },
  });
});

afterEach(() => {
  cleanup();
  restore
    .splice(0)
    .reverse()
    .forEach((dispose) => dispose());
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}

function prepareMarketCache() {
  for (const [storage, key, value] of [
    [localStorage, CACHE_KEY, JSON.stringify({ fetched_at: 1, items: [marketItem], errors: [] })],
    [localStorage, INSTALLED_MARKET_KEY, '{}'],
    [sessionStorage, AUTO_SYNC_KEY, '1'],
  ] as const) {
    const previous = storage.getItem(key);
    storage.setItem(key, value);
    restore.push(() => (previous === null ? storage.removeItem(key) : storage.setItem(key, previous)));
  }
}

function mount() {
  const messageHook = spyOn(arcoMessageHook, 'useArcoMessage').mockReturnValue([
    {
      success: () => () => {},
      warning: () => () => {},
      error: () => () => {},
    },
    null,
  ] as unknown as ReturnType<typeof arcoMessageHook.useArcoMessage>);
  restore.push(() => messageHook.mockRestore());
  return render(h(I18nextProvider, { i18n: locale }, h(SkillMarketSettings)));
}

describe('SkillMarketSettings controlled installation', () => {
  test('sends only market identity, deduplicates pending clicks, and marks the exact item added', async () => {
    prepareMarketCache();
    const request = deferred<{ skill_names: string[] }>();
    const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]);
    const installing = spyOn(ipcBridge.fs.installSkillMarketItem, 'invoke').mockImplementation(() => request.promise);
    restore.push(() => listing.mockRestore());
    restore.push(() => installing.mockRestore());

    const view = mount();
    const add = await view.findByRole('button', { name: common.add });
    await waitFor(() => expect((add as HTMLButtonElement).disabled).toBe(false));
    expect(view.queryByText(marketItem.install_command)).toBeNull();

    fireEvent.click(add);
    fireEvent.click(add);
    expect(installing).toHaveBeenCalledTimes(1);
    expect(installing).toHaveBeenCalledWith({
      source: marketItem.source,
      id: marketItem.id,
      url: marketItem.url,
    });

    await act(async () => request.resolve({ skill_names: ['different-manifest-name'] }));
    await waitFor(() => expect(view.getByRole('button', { name: common.added })).toBeTruthy());
    expect(JSON.parse(localStorage.getItem(INSTALLED_MARKET_KEY) ?? '{}')).toEqual({
      [marketItem.id]: ['different-manifest-name'],
    });
  });

  test('a failed installed-state probe does not permanently disable Add', async () => {
    prepareMarketCache();
    const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockRejectedValue(new Error('catalog offline'));
    const installing = spyOn(ipcBridge.fs.installSkillMarketItem, 'invoke').mockResolvedValue({ skill_names: ['demo'] });
    const logging = spyOn(console, 'error').mockImplementation(() => {});
    restore.push(() => listing.mockRestore());
    restore.push(() => installing.mockRestore());
    restore.push(() => logging.mockRestore());

    const view = mount();
    const add = await view.findByRole('button', { name: common.add });
    await waitFor(() => expect((add as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(add);
    await waitFor(() => expect(installing).toHaveBeenCalledTimes(1));
  });

  test('reconciles persisted market provenance with the managed library', async () => {
    prepareMarketCache();
    localStorage.setItem(
      INSTALLED_MARKET_KEY,
      JSON.stringify({ [marketItem.id]: ['different-manifest-name'] })
    );
    const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([
      {
        name: 'different-manifest-name',
        description: 'Fixture',
        location: 'fixture',
        is_custom: true,
        source: 'custom',
      },
    ]);
    const installing = spyOn(ipcBridge.fs.installSkillMarketItem, 'invoke').mockResolvedValue({ skill_names: [] });
    restore.push(() => listing.mockRestore());
    restore.push(() => installing.mockRestore());

    const view = mount();
    const added = await view.findByRole('button', { name: common.added });
    expect((added as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(added);
    expect(installing).not.toHaveBeenCalled();
  });
});
