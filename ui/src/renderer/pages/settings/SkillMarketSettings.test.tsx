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
import {
  INSTALLED_MARKET_KEY,
  SKILL_MARKET_CACHE_KEY,
} from './skill/skillMarketProvenance';

const AUTO_SYNC_KEY = 'nomifun.skillMarket.autoSynced.v4';
const LEGACY_INSTALLED_MARKET_KEY = 'nomifun.skillMarket.installed.v1';
const marketItem = {
  id: 'clawhub:owner/demo',
  source: 'clawhub' as const,
  rank: 1,
  name: 'Demo Skill',
  description: 'A fixture skill.',
  url: 'https://clawhub.ai/owner/skills/demo',
  install_command: 'openclaw skills install @owner/demo',
};
const sameSlugItem = {
  ...marketItem,
  id: 'clawhub:other-owner/demo',
  name: 'demo',
  url: 'https://clawhub.ai/other-owner/skills/demo',
  install_command: 'openclaw skills install @other-owner/demo',
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

function prepareMarketCache(items = [marketItem]) {
  for (const [storage, key, value] of [
    [localStorage, SKILL_MARKET_CACHE_KEY, JSON.stringify({ fetched_at: 1, items, errors: [] })],
    [localStorage, INSTALLED_MARKET_KEY, JSON.stringify({ version: 2, items: {} })],
    [localStorage, LEGACY_INSTALLED_MARKET_KEY, '{}'],
    [sessionStorage, AUTO_SYNC_KEY, '1'],
  ] as const) {
    const previous = storage.getItem(key);
    storage.setItem(key, value);
    restore.push(() => (previous === null ? storage.removeItem(key) : storage.setItem(key, previous)));
  }
}

function mount(onInstalled?: () => void) {
  const messageHook = spyOn(arcoMessageHook, 'useArcoMessage').mockReturnValue([
    {
      success: () => () => {},
      warning: () => () => {},
      error: () => () => {},
    },
    null,
  ] as unknown as ReturnType<typeof arcoMessageHook.useArcoMessage>);
  restore.push(() => messageHook.mockRestore());
  return render(h(I18nextProvider, { i18n: locale }, h(SkillMarketSettings, { onInstalled })));
}

describe('SkillMarketSettings controlled installation', () => {
  test('sends only market identity, deduplicates pending clicks, and marks the exact item added', async () => {
    prepareMarketCache();
    const request = deferred<{ skill_names: string[] }>();
    const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]);
    const installing = spyOn(ipcBridge.fs.installSkillMarketItem, 'invoke').mockImplementation(() => request.promise);
    let installedNotifications = 0;
    restore.push(() => listing.mockRestore());
    restore.push(() => installing.mockRestore());

    const view = mount(() => {
      installedNotifications += 1;
    });
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
      version: 2,
      items: {
        [marketItem.id]: {
          source: marketItem.source,
          skill_names: ['different-manifest-name'],
          presentation: {
            name: marketItem.name,
            description: marketItem.description,
            tags: [],
            audience_tags: [],
            scenario_tags: [],
          },
        },
      },
    });
    expect(installedNotifications).toBe(1);
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
      JSON.stringify({
        version: 2,
        items: {
          [marketItem.id]: {
            source: marketItem.source,
            skill_names: ['different-manifest-name'],
          },
        },
      })
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

  test('marks only the exact installed market identity when owners share a slug', async () => {
    prepareMarketCache([marketItem, sameSlugItem]);
    localStorage.setItem(
      INSTALLED_MARKET_KEY,
      JSON.stringify({
        version: 2,
        items: {
          [marketItem.id]: {
            source: marketItem.source,
            skill_names: ['demo'],
          },
        },
      })
    );
    const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([
      {
        name: 'demo',
        description: 'Fixture',
        location: 'fixture',
        is_custom: true,
        source: 'custom',
      },
    ]);
    restore.push(() => listing.mockRestore());

    const view = mount();
    const exact = await view.findByTestId('btn-add-market-skill-clawhub-owner-demo');
    const collision = await view.findByTestId('btn-add-market-skill-clawhub-other-owner-demo');
    await waitFor(() => {
      expect(exact.textContent).toContain(common.added);
      expect(collision.textContent).toContain(common.add);
      expect((exact as HTMLButtonElement).disabled).toBe(true);
      expect((collision as HTMLButtonElement).disabled).toBe(false);
    });
  });
});
