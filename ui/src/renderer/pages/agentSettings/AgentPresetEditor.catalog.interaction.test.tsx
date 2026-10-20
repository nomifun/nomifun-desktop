/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeAll, expect, spyOn, test } from 'bun:test';
import { useState } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { appHosting } from '@/common/adapter/appHostingBridge';
import {
  asAgentPresetId, asCapabilityId, asPackageId, createEmptyAgentPresetDocument,
  type AgentCatalogResponse, type AgentPresetDraft, type AgentPresetEditorResponse,
  type CapabilityModuleCatalogItem,
} from '@/common/types/agentPlatform';
import * as taskModels from '@/renderer/hooks/agent/useModelsForTask';
import * as modelProviders from '@/renderer/hooks/agent/useModelProviderList';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';
import AgentPresetEditor from './AgentPresetEditor';
import { actionFallbackName, actionI18nKey, moduleI18nKey } from './model';

const i18n = createInstance();
beforeAll(async () => {
  await i18n.use(initReactI18next).init({
    lng: 'en-US', fallbackLng: 'en-US', initImmediate: false,
    resources: { 'en-US': { translation: { agentSettings: en, common } } },
    interpolation: { escapeValue: false },
  });
});

const moduleItem = (id: string, actions: Array<[string, string]>): CapabilityModuleCatalogItem => ({
  module: { id: asCapabilityId(id) }, display_name: id, description: `${id} catalog fixture`,
  source_package: { id: asPackageId(`nomifun.${id}`), version: '1.0.0' },
  authoring_policy: 'direct', summary_kind: 'tool',
  actions: actions.map(([action_id, effect_class]) => ({ action_id, effect_class, presentation: 'function_tool' })),
  context_schema_refs: [], event_schema_refs: [], required_resource_kinds: [], required_host_ports: [],
  required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
});
const robot = moduleItem('robot', [['robot/vision', 'read_sensitive'], ['robot/motion', 'physical']]);
const collaboration = moduleItem('agent.collaboration', [['agent/delegate', 'read_local'], ['agent/fork', 'read_local']]);
const sessions = moduleItem('session.management', [
  ['session.management/list', 'read_local'], ['session.management/read', 'read_local'],
  ['session.management/execution_status', 'read_local'], ['session.management/execution_cancel', 'read_local'],
  ['session.management/execution_answer_decision', 'read_local'],
]);
const files = moduleItem('workspace.files', [['workspace.files/read', 'read_local']]);
const browser = moduleItem('browser', [['browser/observe', 'read_sensitive']]);
const modules = [robot, collaboration, sessions, files, browser];
const catalog: AgentCatalogResponse = {
  modules,
  capabilities: modules.map(module => ({
    capability: module.module, kind: 'tool', display_name: module.display_name, description: module.description,
    source_package: module.source_package, source_kind: 'bundled', materialization_state: 'materialized',
    supported_surfaces: ['desktop'], required_runtime_features: [], required_resource_kinds: [],
    required_capabilities: [], conflicting_capabilities: [], action_count: module.actions.length, context_contributor_count: 0,
  })),
  skills: [], mcp_tools: [], roles: [],
};
const presetId = asAgentPresetId('019b0000-0000-7000-8000-000000000081');
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });

function mount(source: AgentPresetEditorResponse['preset']['source'], templateKey: AgentPresetDraft['source_template_key']) {
  const models = spyOn(taskModels, 'useModelsForTask').mockReturnValue({ groups: [], isLoading: false, refresh: () => {} });
  const providers = spyOn(modelProviders, 'useProvidersQuery').mockReturnValue({
    data: [], error: undefined, isLoading: false, isValidating: false, mutate: async () => [],
  });
  const apps = spyOn(appHosting.mcpSources, 'invoke').mockResolvedValue([]);
  for (const spy of [models, providers, apps]) restores.push(() => spy.mockRestore());
  const original: AgentPresetDraft = {
    preset_id: presetId, source_template_key: templateKey, display_name: 'Editable Agent',
    document: createEmptyAgentPresetDocument(),
  };
  const editor: AgentPresetEditorResponse = {
    preset: { preset_id: presetId, source, display_name: original.display_name, bound_target_count: 0 },
    draft: original,
  };
  let current = original;
  const Harness = () => {
    const [draft, setDraft] = useState(original);
    return <AgentPresetEditor editor={editor} draft={draft} catalog={catalog} busyAction={null}
      dirty={draft !== original} onDraftChange={next => { current = next; setDraft(next); }}
      onSave={() => {}} onStartConversation={() => {}} />;
  };
  const view = render(<I18nextProvider i18n={i18n}>
    <SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false, fallback: { 'apps.mcp-sources': [] } }}>
      <MemoryRouter><Harness /></MemoryRouter>
    </SWRConfig>
  </I18nextProvider>);
  const name = (module: CapabilityModuleCatalogItem) => {
    const key = moduleI18nKey(String(module.module.id));
    return key ? i18n.t(`agentSettings.modules.${key}.name`, { defaultValue: module.display_name }) : module.display_name;
  };
  const card = (module: CapabilityModuleCatalogItem) => view.getByRole('heading', { name: name(module), level: 3 }).closest('article')!;
  const toggle = (module: CapabilityModuleCatalogItem, selected: boolean) => view.getByRole('switch', {
    name: i18n.t(selected ? 'agentSettings.workbench.disableModule' : 'agentSettings.workbench.enableModule', { name: name(module) }),
  }) as HTMLButtonElement;
  const disclosure = (module: CapabilityModuleCatalogItem) => card(module).querySelector<HTMLButtonElement>('button[aria-controls]')!;
  const expand = (module: CapabilityModuleCatalogItem) => {
    if (disclosure(module).getAttribute('aria-expanded') !== 'true') fireEvent.click(disclosure(module));
  };
  const action = (module: CapabilityModuleCatalogItem, id: string) => within(card(module)).getByRole('checkbox', {
    name: i18n.t('agentSettings.workbench.toggleAction', {
      action: i18n.t(`agentSettings.actionLabels.${actionI18nKey(id)}`, { defaultValue: actionFallbackName(id) }),
      module: name(module),
    }),
  }) as HTMLInputElement;
  const search = (value: string) => fireEvent.input(view.getByRole('searchbox', { name: en.workbench.searchModules }), {
    target: { value },
  });
  return {
    ...view, card, toggle, disclosure, expand, action, search,
    grant: (module: CapabilityModuleCatalogItem) => current.document.enabled_capabilities.find(entry => entry.capability.id === module.module.id),
    state: () => current,
  };
}

test('the official Companion can explicitly enable and customize Robot from the shared catalog', () => {
  const view = mount('official', 'companion.default');
  expect(view.getByText(i18n.t('agentSettings.workbench.results', { count: modules.length })).textContent).toContain(String(modules.length));
  expect(view.card(robot).textContent).toContain(en.modules.robot.description);
  expect(view.toggle(robot, false).disabled).toBe(false);
  expect(view.state().document.enabled_capabilities).toEqual([]);
  fireEvent.click(view.toggle(robot, false));
  expect(view.toggle(robot, true).getAttribute('aria-checked')).toBe('true');
  view.expand(robot);
  expect(view.action(robot, 'robot/vision').checked).toBe(true);
  expect(view.action(robot, 'robot/motion').disabled).toBe(false);
  fireEvent.click(view.action(robot, 'robot/motion'));
  expect(view.grant(robot)?.action_allowlist).toContain('robot/motion');
});

for (const source of ['official', 'user'] as const) test(`${source} Companion Agent can edit fork and full session actions with catalog-wide search and counts`, () => {
  const view = mount(source, 'companion.default');
  expect(view.getByText(i18n.t('agentSettings.workbench.results', { count: modules.length })).textContent).toContain(String(modules.length));
  for (const module of modules) {
    expect(view.card(module)).toBeTruthy();
    expect(view.toggle(module, false).disabled).toBe(false);
  }

  for (const module of [collaboration, sessions]) {
    fireEvent.click(view.toggle(module, false));
    view.expand(module);
    expect(view.disclosure(module).textContent).toBe(i18n.t('agentSettings.workbench.actionCount', { count: module.actions.length }));
    expect(within(view.card(module)).getAllByRole('checkbox')).toHaveLength(module.actions.length);
    for (const item of module.actions) expect(view.action(module, item.action_id).disabled).toBe(false);
  }
  fireEvent.click(view.action(collaboration, 'agent/fork'));
  expect(view.action(collaboration, 'agent/fork').checked).toBe(false);
  expect(view.action(collaboration, 'agent/fork').disabled).toBe(false);
  fireEvent.click(view.action(collaboration, 'agent/fork'));
  expect(view.grant(collaboration)?.action_allowlist).toContain('agent/fork');
  fireEvent.click(view.action(sessions, 'session.management/list'));
  expect(view.action(sessions, 'session.management/list').checked).toBe(false);
  fireEvent.click(view.action(sessions, 'session.management/list'));
  expect(view.grant(sessions)?.action_allowlist).toContain('session.management/list');

  view.search('agent/fork');
  expect(view.getByText(i18n.t('agentSettings.workbench.results', { count: 1 })).textContent).toContain('1');
  expect(view.action(collaboration, 'agent/fork').checked).toBe(true);
  view.search('session.management/list');
  expect(view.getByText(i18n.t('agentSettings.workbench.results', { count: 1 })).textContent).toContain('1');
  expect(view.disclosure(sessions).textContent).toBe(i18n.t('agentSettings.workbench.actionCount', { count: sessions.actions.length }));
  view.search('workspace.files/read');
  expect(view.card(files)).toBeTruthy();
});
