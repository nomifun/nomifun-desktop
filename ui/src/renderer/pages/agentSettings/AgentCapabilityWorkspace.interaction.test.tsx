import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { Modal } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import {
  asCapabilityId,
  asPackageId,
  createEmptyAgentPresetDocument,
  type AgentCatalogResponse,
  type AgentPresetDocument,
  type CapabilityCatalogItem,
  type CapabilityModuleCatalogItem,
} from '@/common/types/agentPlatform';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';
import AgentCapabilityWorkspace from './AgentCapabilityWorkspace';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  fallbackLng: 'en-US',
  resources: { 'en-US': { translation: { agentSettings: en, common } } },
  interpolation: { escapeValue: false },
});

const moduleItem = (
  id: string,
  actions: Array<[string, string]> = [[`${id}/read`, 'read_local']],
  resources: string[] = []
): CapabilityModuleCatalogItem => ({
  module: { id: asCapabilityId(id), version: '1.0.0' },
  display_name: id,
  description: `${id} description`,
  source_package: { id: asPackageId(`nomifun.${id}`), version: '1.0.0' },
  authoring_policy: 'direct',
  summary_kind: 'tool',
  actions: actions.map(([action_id, effect_class]) => ({
    action_id,
    effect_class,
    input_schema: `schema://${action_id}/input`,
    output_schema: `schema://${action_id}/output`,
    presentation: 'function_tool',
  })),
  context_schema_refs: [],
  event_schema_refs: [],
  required_resource_kinds: resources,
  required_host_ports: [],
  required_modules: [],
  conflicting_modules: [],
  supported_surfaces: ['desktop'],
});

const availability = (
  module: CapabilityModuleCatalogItem,
  available = true
): CapabilityCatalogItem => ({
  capability: module.module,
  kind: module.summary_kind,
  display_name: module.display_name,
  description: module.description,
  source_package: module.source_package,
  source_kind: 'bundled',
  materialization_state: available ? 'materialized' : 'unavailable',
  unavailable_code: available ? undefined : 'CAPABILITY_UNAVAILABLE_ON_PLATFORM',
  supported_surfaces: module.supported_surfaces,
  required_runtime_features: [],
  required_resource_kinds: module.required_resource_kinds,
  required_capabilities: module.required_modules,
  conflicting_capabilities: module.conflicting_modules,
  action_count: module.actions.length,
  context_contributor_count: module.context_schema_refs.length,
});

const files = moduleItem('workspace.files', [
  ['workspace.files/read', 'read_local'],
  ['workspace.files/write', 'write_durable'],
], ['workspace']);
const knowledge = moduleItem('knowledge', [['knowledge/search', 'read_sensitive']], ['knowledge_base']);
const browser = moduleItem('browser', [['browser/observe', 'read_sensitive']]);
const unavailableComputer = moduleItem('computer', [['computer/observe', 'read_sensitive']], ['computer']);

const catalog = (modules = [files, knowledge, browser, unavailableComputer]): AgentCatalogResponse => ({
  modules,
  capabilities: modules.map((module) => availability(module, module !== unavailableComputer)),
  skills: [],
  mcp_tools: [],
  roles: [],
});

const documentWith = (
  entries: Array<[CapabilityModuleCatalogItem['module'], string[]]>
): AgentPresetDocument => ({
  ...createEmptyAgentPresetDocument(),
  enabled_capabilities: entries.map(([capability, action_allowlist]) => ({
    capability,
    action_allowlist,
  })),
});

function mount(
  initialDocument: AgentPresetDocument,
  currentCatalog = catalog(),
  disabled = false
) {
  let current = initialDocument;
  const Harness = () => {
    const [value, setValue] = useState(initialDocument);
    return (
      <AgentCapabilityWorkspace
        document={value}
        catalog={currentCatalog}
        disabled={disabled}
        onChange={(next) => {
          current = next;
          setValue(next);
        }}
      />
    );
  };
  const result = render(
    <MemoryRouter>
      <I18nextProvider i18n={testI18n}><Harness /></I18nextProvider>
    </MemoryRouter>
  );
  return { ...result, state: () => current };
}

afterEach(async () => {
  await act(async () => {
    Modal.destroyAll();
  });
  cleanup();
  document.body.replaceChildren();
});

describe('Agent capability Module workbench', () => {
  test('shows an honest empty catalog state', () => {
    const screen = mount(documentWith([]), catalog([]));
    expect(screen.getByRole('status').textContent).toContain(en.workbench.noModules);
    expect(screen.getByText(en.workbench.noModulesHint)).toBeTruthy();
  });

  test('shows one searchable Module catalog without the legacy transfer controls', () => {
    const screen = mount(documentWith([[files.module, ['workspace.files/read']]]));
    expect(screen.getByRole('region', { name: en.workbench.moduleCatalog })).toBeTruthy();
    expect(screen.getByRole('switch', { name: 'Disable Workspace files' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('switch', { name: 'Enable Browser' }).getAttribute('aria-checked')).toBe('false');
    expect(screen.queryByText('Move in')).toBeNull();
    expect(screen.queryByText('Move out')).toBeNull();
    expect(screen.queryByRole('combobox')).toBeNull();
  });

  test('enables a Module with safe exact actions, then explicitly grants a write action', async () => {
    const screen = mount(documentWith([]));
    await act(async () => { fireEvent.click(screen.getByRole('switch', { name: 'Enable Workspace files' })); });
    await waitFor(() => expect(screen.state().enabled_capabilities).toEqual([{
      capability: files.module,
      action_allowlist: ['workspace.files/read'],
    }]));
    const write = screen.getByRole('checkbox', { name: 'Allow Write in Workspace files' });
    expect((write as HTMLInputElement).checked).toBe(false);
    fireEvent.click(write);
    expect(screen.state().enabled_capabilities[0].action_allowlist).toEqual([
      'workspace.files/read', 'workspace.files/write',
    ]);
  });

  test('filters by category and searches exact actions and resources', () => {
    const screen = mount(documentWith([]));
    fireEvent.click(screen.getByRole('button', { name: /Knowledge & memory/ }));
    expect(screen.getByRole('switch', { name: 'Enable Knowledge' })).toBeTruthy();
    expect(screen.queryByRole('switch', { name: 'Enable Browser' })).toBeNull();
    fireEvent.input(screen.getByRole('searchbox', { name: en.workbench.searchModules }), {
      target: { value: 'workspace.files/write' },
    });
    expect(screen.getByText(en.workbench.noModuleResults)).toBeTruthy();
    fireEvent.click(screen.getAllByRole('button', { name: en.workbench.clearFilters })[0]);
    fireEvent.input(screen.getByRole('searchbox', { name: en.workbench.searchModules }), {
      target: { value: 'workspace' },
    });
    expect(screen.getByRole('switch', { name: 'Enable Workspace files' })).toBeTruthy();
  });

  test('shows resource binding status without authoring a concrete resource', async () => {
    const screen = mount(documentWith([[files.module, ['workspace.files/read']]]));
    fireEvent.click(screen.getByRole('button', { name: /2 actions and contributions/ }));
    expect(await screen.findByText(en.resources.bindingStatus)).toBeTruthy();
    expect(screen.getByText('Workspace')).toBeTruthy();
    expect('resource_bindings' in screen.state()).toBe(false);
  });

  test('keeps an unavailable Module inspectable but cannot enable it', () => {
    const screen = mount(documentWith([]));
    const toggle = screen.getByRole('switch', { name: 'Enable Computer' }) as HTMLButtonElement;
    expect(toggle.disabled).toBe(true);
    expect(screen.getByText(en.common.unavailable)).toBeTruthy();
    expect(screen.state().enabled_capabilities).toEqual([]);
  });

  test('preserves a missing exact grant until the user explicitly disables it', async () => {
    const missing = { id: asCapabilityId('plugin.removed'), version: '3.0.0' };
    const screen = mount(documentWith([[missing, ['plugin.removed/run']]]));
    expect(screen.getByText('plugin.removed')).toBeTruthy();
    expect(screen.getAllByText(en.workbench.missingReason)).toHaveLength(2);
    fireEvent.click(screen.getByRole('switch', { name: 'Disable plugin.removed' }));
    await waitFor(() => expect(screen.state().enabled_capabilities).toEqual([]));
  });

  test('surfaces and removes an action no longer in the canonical Module manifest', () => {
    const screen = mount(documentWith([[
      files.module,
      ['workspace.files/read', 'workspace.files/retired'],
    ]]));
    fireEvent.click(screen.getByRole('button', { name: /3 actions and contributions/ }));
    expect(screen.getByText(en.workbench.actionUnavailable)).toBeTruthy();
    fireEvent.click(screen.getByRole('checkbox', {
      name: 'Remove unavailable action workspace.files/retired',
    }));
    expect(screen.state().enabled_capabilities[0].action_allowlist).toEqual(['workspace.files/read']);
  });
});
