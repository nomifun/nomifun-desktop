import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, render, waitFor, within } from '@testing-library/react';
import { createElement as h } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { SkillInfo } from '@/common/types/skill';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import * as arcoMessageHook from '@/renderer/utils/ui/useArcoMessage';
import {
  INSTALLED_MARKET_KEY,
  SKILL_MARKET_CACHE_KEY,
} from './skill/skillMarketProvenance';
import SkillsHubSettings from './SkillsHubSettings';

const locale = createInstance();
const restore: Array<() => void> = [];
const LEGACY_INSTALLED_MARKET_KEY = 'nomifun.skillMarket.installed.v1';

beforeAll(async () => {
  await locale.init({
    lng: 'en-US',
    resources: { 'en-US': { translation: { common, settings } } },
  });
});

afterEach(() => {
  cleanup();
  localStorage.removeItem(SKILL_MARKET_CACHE_KEY);
  localStorage.removeItem(INSTALLED_MARKET_KEY);
  localStorage.removeItem(LEGACY_INSTALLED_MARKET_KEY);
  restore
    .splice(0)
    .reverse()
    .forEach((dispose) => dispose());
});

const skill = (name: string, description = `${name} description`): SkillInfo => ({
  name,
  description,
  location: `C:/skills/${name}/SKILL.md`,
  is_custom: true,
  source: 'custom',
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}

function prepareMocks() {
  const messageApi = {
    success: () => () => {},
    warning: () => () => {},
    error: () => () => {},
  };
  const messageHook = spyOn(arcoMessageHook, 'useArcoMessage').mockReturnValue([
    messageApi,
    null,
  ] as unknown as ReturnType<typeof arcoMessageHook.useArcoMessage>);
  const listing = spyOn(ipcBridge.fs.listAvailableSkills, 'invoke').mockResolvedValue([]);
  const paths = spyOn(ipcBridge.fs.getSkillPaths, 'invoke').mockResolvedValue({
    user_skills_dir: 'C:/skills',
    builtin_skills_dir: 'C:/builtin-skills',
  });
  const autoSkills = spyOn(ipcBridge.fs.listBuiltinAutoSkills, 'invoke').mockResolvedValue([]);
  restore.push(
    () => messageHook.mockRestore(),
    () => listing.mockRestore(),
    () => paths.mockRestore(),
    () => autoSkills.mockRestore()
  );
  return { listing, paths, autoSkills };
}

function page(active: boolean, refreshToken: number) {
  return h(
    I18nextProvider,
    { i18n: locale },
    h(
      MemoryRouter,
      { initialEntries: ['/settings/skills'] },
      h(SkillsHubSettings, { active, refreshToken })
    )
  );
}

describe('SkillsHubSettings active refresh', () => {
  test('does not load while inactive and renders the latest library after activation', async () => {
    const { listing, paths, autoSkills } = prepareMocks();
    const view = render(page(false, 0));

    await act(async () => {});
    expect(listing).toHaveBeenCalledTimes(0);
    expect(paths).toHaveBeenCalledTimes(0);
    expect(autoSkills).toHaveBeenCalledTimes(0);

    listing.mockResolvedValue([skill('newly-installed')]);
    await act(async () => view.rerender(page(true, 0)));

    expect(await view.findByTestId('skill-card-newly-installed')).toBeTruthy();
    expect(listing).toHaveBeenCalledTimes(1);
    expect(paths).toHaveBeenCalledTimes(1);
    expect(autoSkills).toHaveBeenCalledTimes(1);
  });

  test('reloads the active library when its refresh token changes', async () => {
    const { listing } = prepareMocks();
    listing.mockResolvedValue([skill('before-install')]);
    const view = render(page(true, 0));
    expect(await view.findByTestId('skill-card-before-install')).toBeTruthy();

    listing.mockResolvedValue([skill('after-install')]);
    await act(async () => view.rerender(page(true, 1)));

    expect(await view.findByTestId('skill-card-after-install')).toBeTruthy();
    expect(view.queryByTestId('skill-card-before-install')).toBeNull();
    expect(listing).toHaveBeenCalledTimes(2);
  });

  test('still renders the installed library when auxiliary metadata fails', async () => {
    const { listing, paths, autoSkills } = prepareMocks();
    const logging = spyOn(console, 'error').mockImplementation(() => {});
    restore.push(() => logging.mockRestore());
    listing.mockResolvedValue([skill('installed-without-metadata')]);
    paths.mockRejectedValueOnce(new Error('paths unavailable'));
    autoSkills.mockRejectedValueOnce(new Error('auto metadata unavailable'));

    const view = render(page(true, 0));

    expect(await view.findByTestId('skill-card-installed-without-metadata')).toBeTruthy();
    expect(logging).toHaveBeenCalledTimes(2);
  });

  test('ignores an older request that finishes after a refresh', async () => {
    const { listing } = prepareMocks();
    const firstRequest = deferred<SkillInfo[]>();
    listing
      .mockImplementationOnce(() => firstRequest.promise)
      .mockResolvedValueOnce([skill('fresh-result')]);

    const view = render(page(true, 0));
    await waitFor(() => expect(listing).toHaveBeenCalledTimes(1));

    await act(async () => view.rerender(page(true, 1)));
    expect(await view.findByTestId('skill-card-fresh-result')).toBeTruthy();

    await act(async () => firstRequest.resolve([skill('stale-result')]));
    expect(view.queryByTestId('skill-card-stale-result')).toBeNull();
    expect(view.getByTestId('skill-card-fresh-result')).toBeTruthy();
    expect(listing).toHaveBeenCalledTimes(2);
  });

  test('does not let a request from the inactive tab erase newly installed provenance', async () => {
    const { listing } = prepareMocks();
    const staleRequest = deferred<SkillInfo[]>();
    listing.mockImplementationOnce(() => staleRequest.promise);
    const view = render(page(true, 0));
    await waitFor(() => expect(listing).toHaveBeenCalledTimes(1));

    await act(async () => view.rerender(page(false, 0)));
    const installedState = {
      version: 2,
      items: {
        'skillhub:owner/new-skill': {
          source: 'skillhub',
          skill_names: ['new-skill'],
          presentation: {
            name: 'New Market Skill',
            description: 'New market description.',
            tags: [],
            audience_tags: [],
            scenario_tags: [],
          },
        },
      },
    };
    localStorage.setItem(INSTALLED_MARKET_KEY, JSON.stringify(installedState));

    await act(async () => staleRequest.resolve([]));
    expect(JSON.parse(localStorage.getItem(INSTALLED_MARKET_KEY) ?? '{}')).toEqual(installedState);

    listing.mockResolvedValueOnce([skill('new-skill')]);
    await act(async () => view.rerender(page(true, 1)));
    const card = await view.findByTestId('skill-card-new-skill');
    expect(within(card).getByText('New Market Skill')).toBeTruthy();
  });

  test('renders market presentation metadata without replacing the canonical skill identity', async () => {
    const { listing } = prepareMocks();
    localStorage.setItem(
      INSTALLED_MARKET_KEY,
      JSON.stringify({
        version: 2,
        items: {
          'skillhub:owner/canonical-skill': {
            source: 'skillhub',
            skill_names: ['canonical-skill'],
            presentation: {
              name: 'Market Display Name',
              description: 'Market display description.',
              tags: [],
              audience_tags: [],
              scenario_tags: [],
            },
          },
        },
      })
    );
    listing.mockResolvedValue([
      skill('canonical-skill', 'Canonical backend description that drives the skill runtime.'),
    ]);

    const view = render(page(true, 0));
    const card = await view.findByTestId('skill-card-canonical-skill');

    expect(within(card).getByText('Market Display Name')).toBeTruthy();
    expect(within(card).getByText('Market display description.')).toBeTruthy();
    expect(within(card).queryByText('canonical-skill')).toBeNull();
    expect(within(card).queryByText('Canonical backend description that drives the skill runtime.')).toBeNull();
  });
});
