import '../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import guid from '@/renderer/services/i18n/locales/en-US/guid.json';
import agentSettings from '@/renderer/services/i18n/locales/en-US/agentSettings.json';
import type { ExecutableAgentPreset, AgentSelection } from './types';
import AgentSelector, { type AgentSelectorProps } from './AgentSelector';
import BrowserResourceField from '@/renderer/components/agent/BrowserResourceField';

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
const templates = [
  'chat.minimal',
  'assistant.general',
  'coding.codex',
  'companion.default',
  'creative-studio.default',
  'customer-service.default',
].map((template_key) => ({ template_key }) as OfficialPresetTemplate);
const personalAgents = Array.from({ length: 6 }, (_, index) => ({
  ...saved,
  preset_id: `0190f5fe-7c00-7a00-8000-${String(110 + index).padStart(12, '0')}` as typeof saved.preset_id,
  display_name: `Custom Agent ${index + 1}`,
}));
const LocationProbe = () => {
  const location = useLocation();
  return <output data-testid='location'>{location.pathname}{location.search}</output>;
};
const renderSelector = (props: Partial<AgentSelectorProps> = {}) => {
  const selections: AgentSelection[] = [];
  const Harness = () => {
    const [selection, setSelection] = useState<AgentSelection>(props.selection ?? { kind: 'template', templateKey: 'chat.minimal' });
    const choose = (next: AgentSelection) => { selections.push(next); setSelection(next); };
    return <AgentSelector presets={[saved]} officialTemplates={templates} draftPresets={[draft]} {...props} selection={selection} onSelectTemplate={(templateKey) => choose({ kind: 'template', templateKey })} onSelectPreset={(presetId) => choose({ kind: 'preset', presetId })} />;
  };
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={['/guid']}><Harness /><LocationProbe /></MemoryRouter></I18nextProvider>);
  const open = async () => { await act(async () => { fireEvent.click(page.getByTestId('agent-selector')); }); };
  return { page, open, selections };
};
afterEach(cleanup);

describe('Guid Agent selector', () => {
  test('keeps Agent switching feedback inside the selector', () => {
    const { page } = renderSelector({
      switching: true,
      switchingLabel: 'Switching Agent…',
      selectedLabelOverride: 'Current Agent',
    });
    const trigger = page.getByTestId('agent-selector') as HTMLButtonElement;
    expect(trigger.textContent).toBe('Current Agent');
    expect(trigger.disabled).toBe(true);
    expect(trigger.getAttribute('aria-busy')).toBe('true');
    expect(trigger.title).toBe('Switching Agent…');
    expect(page.queryByRole('dialog')).toBeNull();
  });

  test('supports a current-session label while an Agent switch is pending', () => {
    const { page } = renderSelector({
      disabled: true,
      disabledReason: 'Wait for the current Turn',
      selectedLabelOverride: '当前会话 Agent',
    });
    const trigger = page.getByTestId('agent-selector') as HTMLButtonElement;
    expect(trigger.textContent?.includes('当前会话 Agent')).toBe(true);
    expect(trigger.disabled).toBe(true);
    expect(trigger.title).toBe('Wait for the current Turn');
  });

  test('keeps the collection hidden until the current Agent is opened', async () => {
    const { page, open } = renderSelector();
    expect(page.getByTestId('agent-selector').textContent).toBe(agentSettings.template.chat.minimal.name);
    expect(page.queryByText('Nomi Agent')).toBeNull();
    expect(page.queryByRole('dialog')).toBeNull();
    expect(page.queryByText(saved.display_name)).toBeNull();
    await open();
    expect(page.getByRole('dialog', { name: guid.agentEntries.choose })).not.toBeNull();
    expect(page.getByRole('group', { name: agentSettings.library.mine })).not.toBeNull();
    expect(page.getByRole('group', { name: guid.agentEntries.fromTemplate })).not.toBeNull();
  });

  test('keeps Browser configuration inside the existing Agent menu and opens Chrome setup only on request', async () => {
    let reads = 0;
    const choices: string[] = [];
    const connection = { incarnation: 'chrome-menu', state: 'connected' as const, chromium_major: 144 };
    const { page, open, selections } = renderSelector({
      resourceConfiguration: <BrowserResourceField value='managed-browser'
        client={{ snapshot: async () => { reads += 1; return null; }, connect: async () => connection, disconnect: async () => {} }}
        onChange={(choice) => choices.push(choice)} onReadinessChange={() => {}} />,
    });
    expect(page.queryByText(agentSettings.resources.pickerTitle)).toBeNull();
    expect(reads).toBe(0);
    await open();
    const configuration = page.getByRole('button', { name: agentSettings.resources.pickerTitle });
    expect(configuration.getAttribute('aria-expanded')).toBe('false');
    expect(page.queryByRole('combobox')).toBeNull();
    fireEvent.click(configuration);
    expect(page.getByRole('combobox')).toBeTruthy();
    expect(reads).toBe(0);
    expect(page.queryByText(agentSettings.resources.browser.setup)).toBeNull();
    fireEvent.click(page.getByRole('combobox'));
    await waitFor(() => expect(reads).toBe(1));
    expect(page.getByText(agentSettings.resources.browser.setup)).toBeTruthy();
    await waitFor(() => expect(page.getByText(agentSettings.resources.browser.connect).closest('button')?.disabled).toBe(false));
    fireEvent.click(page.getByText(agentSettings.resources.browser.connect));
    await page.findByText(agentSettings.resources.browser.states.connected);
    const option = await page.findByRole('option', { name: agentSettings.resources.browser.attached });
    // Floating UI dismisses on pointerdown. The nested combobox's portal must
    // remain owned by the menu through both pointerdown and its option click.
    fireEvent.pointerDown(option);
    fireEvent.click(option);
    expect(choices).toEqual(['attached-chrome']);
    expect(page.getByRole('dialog', { name: guid.agentEntries.choose })).toBeTruthy();
    expect(selections).toEqual([]);
    fireEvent.click(configuration);
    expect(page.queryByRole('combobox')).toBeNull();
  });

  test('switches saved Agents, updates the trigger, and closes the menu', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    await act(async () => { fireEvent.click(page.getByRole('button', { name: /Release reviewer/ })); });
    expect(selections).toEqual([{ kind: 'preset', presetId: saved.preset_id }]);
    expect(page.getByTestId('agent-selector').textContent).toBe(saved.display_name);
    expect(page.queryByRole('dialog')).toBeNull();
    await open();
    expect(within(page.getByRole('dialog')).getByRole('button', { name: /Release reviewer/ }).getAttribute('aria-pressed')).toBe('true');
    await act(async () => { fireEvent.click(page.getByRole('button', { name: agentSettings.template.chat.minimal.name })); });
    expect(selections.at(-1)).toEqual({ kind: 'template', templateKey: 'chat.minimal' });
  });

  test('searches custom and official Agent names and descriptions', async () => {
    const { page, open } = renderSelector();
    await open();
    const search = page.getByRole('searchbox');
    await act(async () => { fireEvent.input(search, { target: { value: 'release PLANS' } }); });
    expect(page.getByRole('button', { name: /Release reviewer/ })).not.toBeNull();
    await act(async () => { fireEvent.input(search, { target: { value: agentSettings.template.coding.codex.name } }); });
    expect(page.getByRole('button', { name: agentSettings.template.coding.codex.name })).not.toBeNull();
  });

  test('never exposes dedicated product Agents in Conversation search', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    const search = page.getByRole('searchbox');
    await act(async () => { fireEvent.input(search, { target: { value: agentSettings.template.customerService.default.name } }); });
    expect(page.queryByRole('button', { name: agentSettings.template.customerService.default.name })).toBeNull();
    expect(within(page.getByRole('dialog')).getByRole('status').textContent).toBe(guid.agentEntries.empty);
    await act(async () => { fireEvent.input(search, { target: { value: agentSettings.template.companion.default.name } }); });
    expect(page.queryByRole('button', { name: agentSettings.template.companion.default.name })).toBeNull();
    expect(within(page.getByRole('dialog')).getByRole('status').textContent).toBe(guid.agentEntries.empty);
    expect(within(page.getByRole('dialog')).getByRole('status').textContent).toBe(guid.agentEntries.empty);
    expect(selections).toEqual([]);
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
    expect(document.activeElement?.textContent?.includes(agentSettings.template.chat.minimal.name)).toBe(true);
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    expect(document.activeElement?.textContent?.includes(agentSettings.template.assistant.general.name)).toBe(true);
    await act(async () => { fireEvent.keyDown(document.activeElement!, { key: 'Escape' }); });
    expect(page.queryByRole('dialog')).toBeNull();
  });

  test('official Agents select in place, update the label and checkmark, and close the menu', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    await act(async () => { fireEvent.click(page.getByRole('button', { name: agentSettings.template.assistant.general.name })); });
    expect(page.getByTestId('location').textContent).toBe('/guid');
    expect(selections).toEqual([{ kind: 'template', templateKey: 'assistant.general' }]);
    expect(page.getByTestId('agent-selector').textContent).toBe(agentSettings.template.assistant.general.name);
    expect(page.queryByRole('dialog')).toBeNull();
    await open();
    expect(within(page.getByRole('dialog')).getByRole('button', { name: agentSettings.template.assistant.general.name }).getAttribute('aria-pressed')).toBe('true');
  });

  test('keeps official Agent rows to one visible line while descriptions remain searchable', async () => {
    const { page, open } = renderSelector();
    await open();
    const dialog = within(page.getByRole('dialog'));
    const officialName = agentSettings.template.assistant.general.name;
    const officialDescription = agentSettings.template.assistant.general.description;
    const row = dialog.getByRole('button', { name: officialName });
    expect(row.textContent).toBe(officialName);
    expect(row.textContent?.includes(officialDescription)).toBe(false);

    const search = page.getByRole('searchbox');
    await act(async () => { fireEvent.input(search, { target: { value: officialDescription } }); });
    expect(dialog.getByRole('button', { name: officialName })).not.toBeNull();
  });

  test('groups official Agent choices in the compact two-column grid', async () => {
    const { page, open } = renderSelector();
    await open();
    const officialGroup = page.getByRole('group', { name: guid.agentEntries.fromTemplate });
    expect(officialGroup.children[1]?.tagName).toBe('DIV');
    expect(officialGroup.children[1]?.childElementCount).toBe(4);
    expect(within(officialGroup).getAllByRole('button')).toHaveLength(4);
  });

  test('prioritizes official Agents before the collapsed personal Agent section', async () => {
    const { page, open } = renderSelector({ presets: personalAgents, draftPresets: [draft] });
    await open();
    const dialog = page.getByRole('dialog');
    const officialGroup = page.getByRole('group', { name: guid.agentEntries.fromTemplate });
    const personalGroup = page.getByRole('group', { name: agentSettings.library.mine });
    expect(
      Boolean(officialGroup.compareDocumentPosition(personalGroup) & Node.DOCUMENT_POSITION_FOLLOWING)
    ).toBe(true);
    expect(within(personalGroup).getAllByRole('button', { name: /Custom Agent/ })).toHaveLength(3);
    expect(within(dialog).getByRole('button', { name: guid.agentEntries.showMorePersonal.replace('{{count}}', '4') })).not.toBeNull();
  });

  test('drafts stay discoverable and open the exact editor', async () => {
    const { page, open, selections } = renderSelector();
    await open();
    await act(async () => { fireEvent.click(page.getByRole('button', { name: /New researcher/ })); });
    expect(page.getByTestId('location').textContent).toBe(`/agent?preset=${draft.preset_id}`);
    expect(selections).toEqual([]);
  });

  test('shows every conversation-capable official Agent without an expansion step', async () => {
    const { page, open } = renderSelector();
    await open();
    const dialog = within(page.getByRole('dialog'));
    expect(dialog.getByRole('button', { name: agentSettings.template.coding.codex.name })).not.toBeNull();
    expect(dialog.getByRole('button', { name: agentSettings.template.assistant.general.name })).not.toBeNull();
    expect(dialog.getByRole('button', { name: agentSettings.template.chat.minimal.name })).not.toBeNull();
    expect(dialog.queryByRole('button', { name: agentSettings.template.companion.default.name })).toBeNull();
    expect(dialog.getByRole('button', { name: agentSettings.template.creativeStudio.default.name })).not.toBeNull();
  });

  test('limits custom Agents to three and expands the combined saved and draft list on demand', async () => {
    const { page, open } = renderSelector({ presets: personalAgents, draftPresets: [draft] });
    await open();
    for (const preset of personalAgents.slice(0, 3)) {
      expect(page.getByRole('button', { name: new RegExp(preset.display_name) })).not.toBeNull();
    }
    expect(page.queryByRole('button', { name: /Custom Agent 4/ })).toBeNull();
    expect(page.queryByRole('button', { name: /New researcher/ })).toBeNull();
    fireEvent.click(page.getByRole('button', { name: guid.agentEntries.showMorePersonal.replace('{{count}}', '4') }));
    expect(page.getByRole('button', { name: /Custom Agent 6/ })).not.toBeNull();
    expect(page.getByRole('button', { name: /New researcher/ })).not.toBeNull();
    fireEvent.click(page.getByRole('button', { name: guid.agentEntries.showFewerPersonal }));
    expect(page.queryByRole('button', { name: /Custom Agent 4/ })).toBeNull();
    expect(page.queryByRole('button', { name: /New researcher/ })).toBeNull();
  });

  test('keeps a selected custom Agent visible inside the collapsed three-item list', async () => {
    const selected = personalAgents[5];
    const { page, open } = renderSelector({
      presets: personalAgents,
      draftPresets: [],
      selection: { kind: 'preset', presetId: selected.preset_id },
    });
    await open();
    const selectedRow = within(page.getByRole('dialog')).getByRole('button', {
      name: selected.display_name,
    });
    expect(selectedRow.getAttribute('aria-pressed')).toBe('true');
    expect(within(page.getByRole('dialog')).getAllByRole('button', { name: /Custom Agent/ })).toHaveLength(3);
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
