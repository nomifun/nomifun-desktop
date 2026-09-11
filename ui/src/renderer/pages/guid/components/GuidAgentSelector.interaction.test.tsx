import '../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import guid from '@/renderer/services/i18n/locales/en-US/guid.json';
import agentSettings from '@/renderer/services/i18n/locales/en-US/agentSettings.json';
import type { ExecutableAgentPreset, GuidAgentSelection } from '../types';
import GuidAgentSelector, { type GuidAgentSelectorProps } from './GuidAgentSelector';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US', resources: { 'en-US': { translation: { guid, agentSettings } } },
  interpolation: { escapeValue: false },
});
const saved = {
  preset_id: '0190f5fe-7c00-7a00-8000-000000000101',
  source: 'user', display_name: 'Release reviewer', description: 'Inspect release plans', bound_target_count: 0,
  current_stable_revision: { preset_id: '0190f5fe-7c00-7a00-8000-000000000101', revision: 1, revision_digest: 'a'.repeat(64) },
} as ExecutableAgentPreset;
const draft = { ...saved, preset_id: '0190f5fe-7c00-7a00-8000-000000000102' as typeof saved.preset_id, display_name: 'New researcher', current_stable_revision: undefined };
const templates = ['chat.minimal', 'assistant.general', 'coding.codex'].map((template_key) => ({ template_key }) as OfficialPresetTemplate);
const LocationProbe = () => {
  const location = useLocation();
  return <output data-testid='location'>{location.pathname}{location.search}</output>;
};
const renderSelector = (props: Partial<GuidAgentSelectorProps> = {}) => {
  const selections: GuidAgentSelection[] = [];
  const Harness = () => {
    const [selection, setSelection] = useState<GuidAgentSelection>(props.selection ?? { kind: 'template', templateKey: 'chat.minimal' });
    const choose = (next: GuidAgentSelection) => { selections.push(next); setSelection(next); };
    return <GuidAgentSelector presets={[saved]} officialTemplates={templates} draftPresets={[draft]} {...props} selection={selection} onSelectTemplate={(templateKey) => choose({ kind: 'template', templateKey })} onSelectPreset={(presetId) => choose({ kind: 'preset', presetId })} />;
  };
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={['/guid']}><Harness /><LocationProbe /></MemoryRouter></I18nextProvider>);
  const open = async () => { await act(async () => { fireEvent.click(page.getByTestId('guid-agent-selector')); }); };
  return { page, open, selections };
};
afterEach(cleanup);

describe('Guid Agent selector', () => {
  test('supports a compact current-session label while an Agent switch is pending', () => {
    const { page } = renderSelector({
      compact: true,
      disabled: true,
      selectedLabelOverride: '当前会话 Agent',
    });
    const trigger = page.getByTestId('guid-agent-selector') as HTMLButtonElement;
    expect(trigger.textContent?.includes('当前会话 Agent')).toBe(true);
    expect(trigger.disabled).toBe(true);
  });

  test('keeps the collection hidden until the current Agent is opened', async () => {
    const { page, open } = renderSelector();
    expect(page.getByTestId('guid-agent-selector').textContent).toBe(agentSettings.template.chat.minimal.name);
    expect(page.queryByText('Nomi Agent')).toBeNull();
    expect(page.queryByRole('dialog')).toBeNull();
    expect(page.queryByText(saved.display_name)).toBeNull();
    await open();
    expect(page.getByRole('dialog', { name: guid.agentEntries.choose })).not.toBeNull();
    expect(page.getByRole('group', { name: agentSettings.library.mine })).not.toBeNull();
    expect(page.getByRole('group', { name: guid.agentEntries.fromTemplate })).not.toBeNull();
  });

  test('switches saved Agents, updates the trigger, and closes the menu', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    await act(async () => { fireEvent.click(page.getByRole('button', { name: /Release reviewer/ })); });
    expect(selections).toEqual([{ kind: 'preset', presetId: saved.preset_id }]);
    expect(page.getByTestId('guid-agent-selector').textContent).toBe(saved.display_name);
    expect(page.queryByRole('dialog')).toBeNull();
    await open();
    expect(within(page.getByRole('dialog')).getByRole('button', { name: /Release reviewer/ }).getAttribute('aria-pressed')).toBe('true');
    await act(async () => { fireEvent.click(page.getByRole('button', { name: agentSettings.template.chat.minimal.name })); });
    expect(selections.at(-1)).toEqual({ kind: 'template', templateKey: 'chat.minimal' });
  });

  test('searches names and descriptions, including templates outside the initial preview', async () => {
    const { page, open } = renderSelector();
    await open();
    const search = page.getByRole('searchbox');
    await act(async () => { fireEvent.input(search, { target: { value: 'release PLANS' } }); });
    expect(page.getByRole('button', { name: /Release reviewer/ })).not.toBeNull();
    await act(async () => { fireEvent.input(search, { target: { value: agentSettings.template.coding.codex.name } }); });
    expect(page.getByRole('button', { name: agentSettings.template.coding.codex.name })).not.toBeNull();
  });

  test('Enter chooses a filtered result but does not submit during IME composition', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    const search = page.getByRole('searchbox');
    fireEvent.input(search, { target: { value: 'release' } });
    fireEvent.keyDown(search, { key: 'Enter', isComposing: true });
    expect(selections).toEqual([]);
    await act(async () => { fireEvent.keyDown(search, { key: 'Enter' }); });
    expect(selections).toEqual([{ kind: 'preset', presetId: saved.preset_id }]);
  });

  test('arrow keys move among results and Escape returns to the trigger', async () => {
    const { page, open } = renderSelector();
    await open();
    const search = page.getByRole('searchbox');
    fireEvent.keyDown(search, { key: 'ArrowDown' });
    expect(document.activeElement?.textContent?.includes('Release reviewer')).toBe(true);
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    expect(document.activeElement?.textContent?.includes('New researcher')).toBe(true);
    await act(async () => { fireEvent.keyDown(document.activeElement!, { key: 'Escape' }); });
    expect(page.queryByRole('dialog')).toBeNull();
  });

  test('official Agents select in place, update the label and checkmark, and close the menu', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    await act(async () => { fireEvent.click(page.getByRole('button', { name: agentSettings.template.assistant.general.name })); });
    expect(page.getByTestId('location').textContent).toBe('/guid');
    expect(selections).toEqual([{ kind: 'template', templateKey: 'assistant.general' }]);
    expect(page.getByTestId('guid-agent-selector').textContent).toBe(agentSettings.template.assistant.general.name);
    expect(page.queryByRole('dialog')).toBeNull();
    await open();
    expect(within(page.getByRole('dialog')).getByRole('button', { name: agentSettings.template.assistant.general.name }).getAttribute('aria-pressed')).toBe('true');
  });

  test('drafts stay discoverable and open the exact editor', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    await act(async () => { fireEvent.click(page.getByRole('button', { name: /New researcher/ })); });
    expect(page.getByTestId('location').textContent).toBe(`/agent?preset=${draft.preset_id}`);
    expect(selections).toEqual([]);
  });

  test('all templates can be expanded and collapsed without changing the current Agent', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    expect(page.queryByRole('button', { name: agentSettings.template.coding.codex.name })).toBeNull();
    fireEvent.click(page.getByRole('button', { name: guid.agentEntries.browseTemplates }));
    expect(page.getByRole('button', { name: agentSettings.template.coding.codex.name })).not.toBeNull();
    fireEvent.click(page.getByRole('button', { name: guid.agentEntries.fewerTemplates }));
    expect(page.queryByRole('button', { name: agentSettings.template.coding.codex.name })).toBeNull();
    expect(selections).toEqual([]);
  });

  test('empty search results do not hide management or submit a different Agent', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    const search = page.getByRole('searchbox');
    await act(async () => { fireEvent.input(search, { target: { value: 'no-such-agent' } }); });
    fireEvent.keyDown(search, { key: 'Enter' });
    expect(within(page.getByRole('dialog')).getByRole('status').textContent).toBe(guid.agentEntries.empty);
    expect(page.getByRole('link', { name: guid.agentEntries.manage })).not.toBeNull();
    expect(selections).toEqual([]);
  });

  test('load failures retain cached choices and provide retry', async () => {
    let retries = 0;
    const { page, open } = renderSelector({ loadError: new Error('offline'), onRetry: async () => { retries++; } });
    await open();
    expect(page.getByRole('alert').textContent?.includes(guid.agentEntries.loadFailed)).toBe(true);
    expect(page.getByRole('button', { name: /Release reviewer/ })).not.toBeNull();
    fireEvent.click(page.getByRole('button', { name: agentSettings.actions.retry }));
    expect(retries).toBe(1);
  });
});
