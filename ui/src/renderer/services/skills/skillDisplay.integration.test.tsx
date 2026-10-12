import { afterEach, beforeAll, describe, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import type { ISkillMarketItem } from '@/common/adapter/ipcBridge';
import type { SkillInfo } from '@/common/types/skill';
import SessionCapabilityPicker from '@/renderer/components/chat/SessionCapabilityPicker';
import SkillCard from '@/renderer/pages/settings/skill/SkillCard';
import SkillDetailDrawer from '@/renderer/pages/settings/skill/SkillDetailDrawer';
import SkillMarketCard from '@/renderer/pages/settings/skill/SkillMarketCard';
import CatalogPickerModal from '@/renderer/pages/nomi/workspace/tabs/SkillsTab/CatalogPickerModal';
import CatalogSkillDetail from '@/renderer/pages/nomi/workspace/tabs/SkillsTab/CatalogSkillDetail';
import { buildSkillEntries } from '@/renderer/pages/nomi/workspace/tabs/SkillsTab/unify';
import settings from '@/renderer/services/i18n/locales/en-US/settings.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import { useSkillDisplay } from './useSkillDisplay';
import {
  INSTALLED_MARKET_KEY, SKILL_MARKET_CACHE_KEY,
  notifySkillMarketCacheChanged, readInstalledMarketState,
  recordInstalledMarketItem, writeInstalledMarketState,
} from './skillMarketProvenance';

const locale = createInstance();
beforeAll(async () => {
  await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { settings, common } } } });
});
afterEach(async () => {
  cleanup();
  await locale.changeLanguage('en-US');
  writeInstalledMarketState({});
  localStorage.removeItem(INSTALLED_MARKET_KEY);
  localStorage.removeItem(SKILL_MARKET_CACHE_KEY);
});

const skill: SkillInfo = {
  name: 'dev-expert', description: 'Canonical instructions summary',
  location: 'C:/skills/dev-expert/SKILL.md', source: 'custom', is_custom: true,
};
const item: ISkillMarketItem = {
  id: 'skillhub:owner/dev-expert', source: 'skillhub', rank: 1,
  name: '编程专家.Skill', description: 'GitHub coding helper',
  url: 'https://skillhub.cn/skills/owner/dev-expert',
  install_command: 'npx skills add @owner/dev-expert',
};
const noAuto = new Set<string>();

function CompanionDetail() {
  const display = useSkillDisplay(locale.language);
  const entry = buildSkillEntries({
    generated: [], catalog: [skill], autoNames: noAuto,
    config: { enabled: [skill.name], disabled_auto: [] }, missingDescription: 'missing', display,
  })[0];
  return entry.kind === 'catalog'
    ? <section data-testid='companion-detail'><h3>{entry.name}</h3><CatalogSkillDetail entry={entry} busy={false} disabled={false} onRevoke={() => {}} /></section>
    : null;
}

function Surfaces({ marketItem = item, unavailable = false, onSelect, onGrant }: {
  marketItem?: ISkillMarketItem; unavailable?: boolean;
  onSelect?: (draft: { skillNames: string[]; mcpServerIds: string[] }) => void;
  onGrant?: (name: string, granted: boolean) => void;
}) {
  const [detailOpen, setDetailOpen] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  return <MemoryRouter><I18nextProvider i18n={locale}>
    <section data-testid='market'><SkillMarketCard item={marketItem} adding={false} added addedStateLoading={false} onAdd={() => {}} /></section>
    <SkillCard skill={skill} localeKey={locale.language} isAutoInjected={false} onOpenDetails={() => setDetailOpen(true)} onDelete={() => {}} />
    <SkillDetailDrawer skill={skill} localeKey={locale.language} isAutoInjected={false} visible={detailOpen} onClose={() => setDetailOpen(false)} />
    <SessionCapabilityPicker
      catalog={{ skills: [{ ...skill, auto: false, session_available: !unavailable, session_error: unavailable ? 'Skill exceeds the freeze size limit' : null }], autoSkillNames: noAuto, mcpServers: [] }}
      draft={{ skillNames: [], mcpServerIds: [] }} onChange={onSelect ?? (() => {})} applyMode='create'
    />
    <button onClick={() => setPickerOpen(true)}>Grant picker</button>
    <CatalogPickerModal visible={pickerOpen} onClose={() => setPickerOpen(false)}
      catalog={[skill]} autoNames={noAuto} config={{ enabled: [], disabled_auto: [] }}
      localeKey={locale.language} busyName={null} disabled={false} onToggle={onGrant ?? (() => {})}
    />
    <CompanionDetail />
  </I18nextProvider></MemoryRouter>;
}

function expectCopy(element: HTMLElement, marketItem = item) {
  expect(within(element).getAllByText(marketItem.name).length).toBeGreaterThan(0);
  expect(within(element).getByText(marketItem.description)).toBeTruthy();
  expect(within(element).queryByText(skill.description)).toBeNull();
}

describe('skill display across renderer surfaces', () => {
  test('builtin library cards, Session choices and grant choices follow the same live locale change', async () => {
    const builtin: SkillInfo = {
      ...skill, name: 'creative-studio-canvas', source: 'builtin', is_custom: false,
      name_i18n: { 'zh-CN': '画布规划' }, description_i18n: { 'zh-CN': '来自技能库的完整画布说明。' },
    };
    const surfaces = () => <MemoryRouter><I18nextProvider i18n={locale}>
      <SkillCard skill={builtin} localeKey={locale.language} isAutoInjected={false} onOpenDetails={() => {}} onDelete={() => {}} />
      <SessionCapabilityPicker catalog={{ skills: [{ ...builtin, auto: false }], autoSkillNames: noAuto, mcpServers: [] }} draft={{ skillNames: [], mcpServerIds: [] }} onChange={() => {}} applyMode='create' />
      <CatalogPickerModal visible onClose={() => {}} catalog={[builtin]} autoNames={noAuto} config={{ enabled: [], disabled_auto: [] }} localeKey={locale.language} busyName={null} disabled={false} onToggle={() => {}} />
    </I18nextProvider></MemoryRouter>;
    const view = render(surfaces());
    fireEvent.click(view.getByTestId('session-skills-trigger'));
    expect(within(view.getByTestId('skill-card-creative-studio-canvas')).getByText(builtin.description)).toBeTruthy();
    await act(async () => {
      await locale.changeLanguage('zh-CN');
      view.rerender(surfaces());
    });
    for (const id of ['skill-card-creative-studio-canvas', 'session-skills-list']) {
      expect(within(view.getByTestId(id)).getAllByText('画布规划').length).toBeGreaterThan(0);
      expect(within(view.getByTestId(id)).getByText('来自技能库的完整画布说明。')).toBeTruthy();
    }
    const grant = view.getByRole('switch', { name: '画布规划' });
    expect(within(grant.closest('.arco-modal') as HTMLElement).getByText('来自技能库的完整画布说明。')).toBeTruthy();
  });

  test('live installation metadata reaches cards, Session picker, grants and details with canonical actions intact', async () => {
    const onSelect = mock(() => {});
    const onGrant = mock(() => {});
    const readFile = spyOn(ipcBridge.fs.readFile, 'invoke').mockResolvedValue('# Actual managed instructions');
    try {
      const view = render(<Surfaces onSelect={onSelect} onGrant={onGrant} />);
      expect(view.getByTestId('skill-card-dev-expert').textContent).toContain(skill.description);
      await act(async () => writeInstalledMarketState(recordInstalledMarketItem({}, item, [skill.name])));
      expectCopy(view.getByTestId('market'));
      expectCopy(view.getByTestId('skill-card-dev-expert'));
      expectCopy(view.getByTestId('companion-detail'));

      fireEvent.click(view.getByTestId('session-skills-trigger'));
      expectCopy(await view.findByTestId('session-skills-list'));
      fireEvent.click(view.getByRole('checkbox', { name: item.name }));
      expect(onSelect).toHaveBeenCalledWith({ skillNames: [skill.name], mcpServerIds: [] });
      fireEvent.click(view.getByTestId('session-skills-trigger'));

      fireEvent.click(view.getByText('Grant picker'));
      const grantSwitch = await view.findByRole('switch', { name: item.name });
      expect(within(grantSwitch.closest('.arco-modal') as HTMLElement).getByText(item.description)).toBeTruthy();
      fireEvent.click(grantSwitch);
      expect(onGrant).toHaveBeenCalledWith(skill.name, true);
      fireEvent.click(view.getByRole('button', { name: 'Close' }));

      fireEvent.click(view.getByTestId('skill-card-dev-expert'));
      expectCopy(await view.findByTestId('skill-detail-content'));
      await waitFor(() => expect(readFile).toHaveBeenCalledWith({ path: skill.location }));
      expect(skill.name).toBe('dev-expert');
      expect(skill.description).toBe('Canonical instructions summary');
    } finally { readFile.mockRestore(); }
  });

  test('market refreshes and other-window provenance changes update already-open consumers without navigation', async () => {
    writeInstalledMarketState(recordInstalledMarketItem({}, item, [skill.name]));
    const view = render(<Surfaces />);
    fireEvent.click(view.getByTestId('session-skills-trigger'));
    const refreshed = { ...item, name: 'Updated market title', description: 'Updated authoritative market summary' };
    await act(async () => {
      localStorage.setItem(SKILL_MARKET_CACHE_KEY, JSON.stringify({ items: [refreshed] }));
      notifySkillMarketCacheChanged(SKILL_MARKET_CACHE_KEY);
      view.rerender(<Surfaces marketItem={refreshed} />);
    });
    for (const id of ['market', 'skill-card-dev-expert', 'companion-detail', 'session-skills-list']) expectCopy(view.getByTestId(id), refreshed);

    await act(async () => {
      localStorage.removeItem(SKILL_MARKET_CACHE_KEY);
      localStorage.setItem(INSTALLED_MARKET_KEY, JSON.stringify({ version: 2, items: recordInstalledMarketItem({}, item, [skill.name]) }));
      window.dispatchEvent(new StorageEvent('storage', { key: INSTALLED_MARKET_KEY }));
    });
    expectCopy(view.getByTestId('skill-card-dev-expert'));
    expectCopy(view.getByTestId('session-skills-list'));
  });

  test('unavailable skills retain the shared description and expose the failure separately without enabling selection', async () => {
    writeInstalledMarketState(recordInstalledMarketItem({}, item, [skill.name]));
    const onSelect = mock(() => {});
    const view = render(<Surfaces unavailable onSelect={onSelect} />);
    fireEvent.click(view.getByTestId('session-skills-trigger'));
    expectCopy(await view.findByTestId('session-skills-list'));
    expect(view.getByTitle('Skill exceeds the freeze size limit')).toBeTruthy();
    const checkbox = view.getByRole('checkbox', { name: item.name }) as HTMLInputElement;
    expect(checkbox.disabled).toBe(true);
    fireEvent.click(checkbox);
    expect(onSelect).not.toHaveBeenCalled();
  });

  test('best-effort browser storage failure does not turn a successful install into divergent UI state', async () => {
    const view = render(<Surfaces />);
    const persist = spyOn(localStorage, 'setItem').mockImplementationOnce(() => { throw new Error('Storage full'); });
    try {
      await act(async () => writeInstalledMarketState(recordInstalledMarketItem({}, item, [skill.name])));
      expectCopy(view.getByTestId('skill-card-dev-expert'));
      expectCopy(view.getByTestId('companion-detail'));
      expect(readInstalledMarketState()[item.id]?.skill_names).toEqual([skill.name]);
    } finally { persist.mockRestore(); }
  });
});
