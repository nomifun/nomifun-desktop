import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import type { AgentPresetDocument, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import OfficialTemplateOverview, { documentFromTemplate } from './OfficialTemplateOverview';
import { runtimeEngineOptions } from './AgentRuntimeEngineSelector';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en, common } } } });
afterEach(cleanup);

const template: OfficialPresetTemplate = {
  template_key: 'chat.minimal', immutable: true, forkable: true,
  seed: { enabled_capabilities: [], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] },
  role_coverage: { required_capability_categories: [], required_capability_ids: [], required_runtime_features: [], required_resource_kinds: [] },
};
const engines = ['nomi', 'coding'].map((name) => ({
  family_id: `nomifun.${name}`, build_id: 'windows-test', build_digest: name === 'nomi' ? 'a'.repeat(64) : 'b'.repeat(64),
  host_contract_version: 1, display_name: `${name} Engine`, supported_profiles: ['workflow'],
}));
const options = runtimeEngineOptions(engines);

function mount(busy = false) {
  const saves: AgentPresetDocument[] = [];
  let dirty = false;
  const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{
    provider: () => new Map(), fallback: { 'runtime-engines': engines }, revalidateOnMount: false,
  }}><OfficialTemplateOverview template={template} busy={busy} catalog={{ capabilities: [], skills: [], mcp_tools: [], roles: [] }}
    onSave={(_name, document) => saves.push(structuredClone(document))}
    onDirtyChange={(value) => { dirty = value; }}
  /></SWRConfig></MemoryRouter></I18nextProvider>);
  fireEvent.click(screen.getByRole('tab', { name: en.workbench.settingsTab }));
  return { ...screen, saves, dirty: () => dirty };
}

test('official template can select either bundled engine before saving a personal Agent', async () => {
  const original = structuredClone(template);
  const screen = mount();
  for (const option of options) {
    fireEvent.click(screen.getByRole('combobox', { name: en.runtimeEngine.label }));
    fireEvent.click(await screen.findByText(option.label));
    await waitFor(() => expect(screen.dirty()).toBe(true));
    // Switching tabs must not discard the engine selection.
    fireEvent.click(screen.getByRole('tab', { name: en.workbench.capabilityTab }));
    fireEvent.click(screen.getByRole('tab', { name: en.workbench.settingsTab }));
    fireEvent.click(screen.getByRole('button', { name: en.workbench.saveAsMine }));
    expect(screen.saves.at(-1)).toEqual({ ...documentFromTemplate(template), runtime_engine: option.selection });
  }
  expect(screen.saves).toHaveLength(2);
  expect(template).toEqual(original);
});

test('template reset and default selection both restore the default engine', async () => {
  const screen = mount();
  for (const resetTemplate of [false, true]) {
    fireEvent.click(screen.getByRole('combobox', { name: en.runtimeEngine.label }));
    fireEvent.click(await screen.findByText(options[1].label));
    await waitFor(() => expect(screen.dirty()).toBe(true));
    if (resetTemplate) fireEvent.click(screen.getByRole('button', { name: en.workbench.resetTemplate }));
    else {
      fireEvent.click(screen.getByRole('combobox', { name: en.runtimeEngine.label }));
      fireEvent.click(await screen.findByText(en.runtimeEngine.default));
    }
    await waitFor(() => expect(screen.dirty()).toBe(false));
    fireEvent.click(screen.getByRole('button', { name: en.workbench.saveAsMine }));
    expect(screen.saves.at(-1)?.runtime_engine).toBeUndefined();
  }
});

test('busy official template cannot change engines or save', () => {
  const screen = mount(true);
  const select = screen.getByRole('combobox', { name: en.runtimeEngine.label });
  expect(select.getAttribute('aria-disabled')).toBe('true');
  expect((screen.getByRole('button', { name: en.workbench.saveAsMine }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(select);
  expect(screen.queryByRole('option')).toBeNull();
  expect(screen.saves).toHaveLength(0);
  expect(screen.dirty()).toBe(false);
});
