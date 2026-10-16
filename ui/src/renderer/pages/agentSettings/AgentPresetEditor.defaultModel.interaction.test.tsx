import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { useState } from 'react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { agentPlatform } from '@/common/adapter/ipcBridge';
import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import {
  AGENT_CHAT_MODEL_TASK,
  CHAT_ROUTE_RECORD_SCHEMA,
  asAgentPresetId,
  asCapabilityId,
  asDigestHex,
  asPackageId,
  createEmptyAgentPresetDocument,
  type AgentCatalogResponse,
  type AgentPresetDraft,
  type AgentPresetEditorResponse,
  type ChatRouteRecord,
} from '@/common/types/agentPlatform';
import * as taskModels from '@/renderer/hooks/agent/useModelsForTask';
import * as modelProviders from '@/renderer/hooks/agent/useModelProviderList';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';
import AgentPresetEditor from './AgentPresetEditor';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { agentSettings: en, common } } },
  interpolation: { escapeValue: false },
});

const providerAId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000301');
const providerBId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000302');
const provider = (id: IProvider['id'], name: string, models: [string, string][]): IProvider => ({
  id,
  name,
  platform: 'custom',
  enabled: true,
  base_url: 'https://example.invalid/v1',
  auth_scheme: 'bearer',
  has_credentials: true,
  models: models.map(([model, display_name], sort_order) => ({
    provider_id: id,
    model,
    display_name,
    enabled: true,
    sort_order,
    capabilities: [{
      task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default',
      allow_cross_origin_credentials: false, provider_params: {}, created_at: 1, updated_at: 1,
    }],
    created_at: 1,
    updated_at: 1,
  })),
});
const providers = [
  provider(providerAId, 'First provider', [['old-model', 'Old model'], ['first-choice', 'First choice']]),
  provider(providerBId, 'Second provider', [['next-model', 'Next model'], ['latest-model', 'Latest model']]),
];
const route = (providerId: IProvider['id'], model: string, revision = 7): ChatRouteRecord => ({
  schema: CHAT_ROUTE_RECORD_SCHEMA,
  task: AGENT_CHAT_MODEL_TASK,
  primary: {
    model_route_id: `host-route-${model}`,
    model_route_revision: revision,
    provider_id: providerId,
    model,
    protocol: 'openai_responses',
    connection_config_ref: `host-config-${model}`,
    config_revision_digest: asDigestHex('c'.repeat(64)),
    credential_ref: `host-credential-${model}`,
    features: ['text_input', 'text_output', 'tool_calls', 'native_responses_items'],
  },
  failovers: [],
});
const initialRoute = route(providerAId, 'old-model', 3);
const presetId = asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000303');
const revision = { preset_id: presetId, revision: 4, revision_digest: asDigestHex('b'.repeat(64)) };
const capability = { id: asCapabilityId('workspace.files') };
const sourcePackage = { id: asPackageId('test'), version: '1.0.0' };
const catalog: AgentCatalogResponse = {
  modules: [{
    module: capability, display_name: 'Workspace files', description: '', source_package: sourcePackage,
    authoring_policy: 'direct', summary_kind: 'tool',
    actions: [{ action_id: 'workspace.files/read', effect_class: 'read_local', presentation: 'function_tool' }],
    context_schema_refs: [], event_schema_refs: [], required_resource_kinds: [], required_host_ports: [],
    required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
  }],
  capabilities: [{
    capability, kind: 'tool', display_name: 'Workspace files', description: '', source_package: sourcePackage,
    source_kind: 'bundled', materialization_state: 'materialized', supported_surfaces: ['desktop'],
    required_runtime_features: [], required_resource_kinds: [], required_capabilities: [],
    conflicting_capabilities: [], action_count: 1, context_contributor_count: 0,
  }],
  skills: [], mcp_tools: [], roles: [],
};

afterEach(() => { cleanup(); mock.restore(); });

function mount(defaultRoute: ChatRouteRecord | null = initialRoute, hasRevision = true, availableProviders = providers) {
  const modelCatalog = spyOn(taskModels, 'useModelsForTask').mockReturnValue({
    groups: availableProviders.map(provider => ({ provider, models: provider.models.map(model => model.model) })),
    isLoading: false,
    refresh: () => {},
  });
  const providerCatalog = spyOn(modelProviders, 'useProvidersQuery').mockReturnValue({
    data: availableProviders, error: undefined, isLoading: false, isValidating: false, mutate: async () => availableProviders,
  });
  const presetDocument = createEmptyAgentPresetDocument();
  presetDocument.enabled_capabilities = [{ capability, action_allowlist: ['workspace.files/read'] }];
  presetDocument.model_route_refs.unrelated_task = 'unrelated-route';
  if (defaultRoute) {
    presetDocument.model_route_refs[AGENT_CHAT_MODEL_TASK] = defaultRoute.primary.model_route_id;
    presetDocument.chat_route_records[AGENT_CHAT_MODEL_TASK] = structuredClone(defaultRoute);
  }
  const original: AgentPresetDraft = {
    preset_id: presetId,
    source_template_key: 'assistant.general',
    display_name: 'Official Agent',
    current_revision: hasRevision ? revision : undefined,
    document: presetDocument,
  };
  const editor: AgentPresetEditorResponse = {
    preset: {
      preset_id: presetId, source: 'official',
      display_name: original.display_name, bound_target_count: 1,
      current_stable_revision: hasRevision ? revision : undefined,
    },
    draft: original,
    revision: hasRevision
      ? { reference: revision, document: original.document, created_by: 'owner', created_at_ms: 1 }
      : undefined,
  };
  let current = original;
  let saved: AgentPresetDraft | undefined;
  const Harness = () => {
    const [draft, setDraft] = useState(original);
    return <AgentPresetEditor editor={editor} draft={draft} catalog={catalog}
      busyAction={null} dirty={draft !== original} onDraftChange={next => { current = next; setDraft(next); }}
      onSave={() => { saved = structuredClone(draft); }} onStartConversation={() => {}} />;
  };
  const content = () => <I18nextProvider i18n={i18n}>
    <SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false, fallback: { 'apps.mcp-sources': [] } }}>
      <MemoryRouter><Harness /></MemoryRouter>
    </SWRConfig>
  </I18nextProvider>;
  const view = render(content());
  const refreshCatalog = (next: IProvider[]) => {
    modelCatalog.mockReturnValue({ groups: next.map(provider => ({ provider, models: provider.models.map(model => model.model) })),
      isLoading: false, refresh: () => {} });
    providerCatalog.mockReturnValue({ data: next, error: undefined, isLoading: false, isValidating: false, mutate: async () => next });
    view.rerender(content());
  };
  fireEvent.click(view.getByRole('tab', { name: en.workbench.settingsTab }));
  const picker = () => {
    const target = view.getByTestId('agent-default-model-picker');
    return target.matches('.arco-select') ? target : target.querySelector('.arco-select')!;
  };
  const choose = async (label: string) => {
    fireEvent.click(picker());
    const option = await within(document.body).findByText(label);
    await act(async () => {
      fireEvent.click(option);
      // Arco closes a single-select popup on the next timer tick.
      await new Promise(resolve => setTimeout(resolve, 0));
    });
  };
  const clear = () => {
    fireEvent.mouseEnter(picker());
    const icon = picker().querySelector('.arco-select-clear-icon');
    expect(icon).not.toBeNull();
    fireEvent.click(icon!);
  };
  const save = () => {
    const button = view.getByRole('button', { name: common.save }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    fireEvent.click(button);
    expect(saved).toBeDefined();
    return saved!;
  };
  return { ...view, picker, choose, clear, save, refreshCatalog, state: () => current, original };
}

test('selects a configured model outside saved failovers and saves the host record on the same official Agent', async () => {
  const resolved = route(providerBId, 'next-model', 19);
  resolved.failovers = [route(providerAId, 'first-choice', 11).primary];
  const resolve = spyOn(agentPlatform.resolveChatModelRoute, 'invoke').mockResolvedValue(resolved);
  const view = mount();
  expect(view.original.document.chat_route_records.agent_chat?.failovers).toEqual([]);
  await view.choose('Second provider / Next model');
  await waitFor(() => expect(view.state().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(resolved));
  expect(resolve).toHaveBeenCalledWith({ preset_id: presetId, model: { provider_id: providerBId, model: 'next-model' } });
  const saved = view.save();
  expect(saved.document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(resolved);
  expect(saved.document.model_route_refs).toEqual({ unrelated_task: 'unrelated-route', agent_chat: resolved.primary.model_route_id });
  expect(saved.preset_id).toBe(presetId);
  expect(saved.source_template_key).toBe('assistant.general');
  expect(saved.current_revision).toEqual(revision);
  expect(saved.document.enabled_capabilities).toEqual(view.original.document.enabled_capabilities);
  expect(view.original.document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(initialRoute);
});

test('clears both default model references and saves without requiring another model', () => {
  const resolve = spyOn(agentPlatform.resolveChatModelRoute, 'invoke');
  const view = mount();
  view.clear();
  const saved = view.save();
  expect(saved.document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toBeUndefined();
  expect(saved.document.model_route_refs).toEqual({ unrelated_task: 'unrelated-route' });
  expect(view.picker().textContent).toContain(en.fields.defaultModelPlaceholder);
  expect(saved.current_revision).toEqual(revision);
  expect(saved.document.enabled_capabilities).toEqual(view.original.document.enabled_capabilities);
  expect(resolve).not.toHaveBeenCalled();
});

test('an Agent with no default model can choose any configured chat model and save it', async () => {
  const resolved = route(providerAId, 'first-choice');
  const resolve = spyOn(agentPlatform.resolveChatModelRoute, 'invoke').mockResolvedValue(resolved);
  const view = mount(null, false);
  expect(view.picker().classList.contains('arco-select-disabled')).toBe(false);
  expect(view.picker().textContent).toContain(en.fields.defaultModelPlaceholder);
  expect(view.save().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toBeUndefined();
  await view.choose('First provider / First choice');
  await waitFor(() => expect(view.state().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(resolved));
  expect(resolve).toHaveBeenCalledWith({ preset_id: presetId, model: { provider_id: providerAId, model: 'first-choice' } });
  expect(view.save().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(resolved);
});

test('an empty Agent default stays empty when the model catalog loads, reorders or disappears', () => {
  const resolve = spyOn(agentPlatform.resolveChatModelRoute, 'invoke');
  const view = mount(null, false, []);
  for (const next of [providers, [...providers].reverse(), [], providers]) {
    view.refreshCatalog(next);
    expect(view.picker().textContent).toContain(en.fields.defaultModelPlaceholder);
    expect(view.picker().textContent).not.toContain('First provider /');
    expect(view.picker().textContent).not.toContain('Second provider /');
    expect(view.state().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toBeUndefined();
    expect(view.state().document.model_route_refs[AGENT_CHAT_MODEL_TASK]).toBeUndefined();
  }
  const saved = view.save();
  expect(saved.document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toBeUndefined();
  expect(saved.document.model_route_refs).toEqual({ unrelated_task: 'unrelated-route' });
  expect(resolve).not.toHaveBeenCalled();
});

test('catalog removal never replaces an explicit Agent default with another available model', () => {
  const resolve = spyOn(agentPlatform.resolveChatModelRoute, 'invoke');
  const view = mount();
  view.refreshCatalog([providers[1]]);
  expect(view.state().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(initialRoute);
  expect(view.picker().textContent).not.toContain('Second provider / Next model');
  fireEvent.change(view.getByRole('textbox', { name: en.fields.persona }), { target: { value: 'Keep this explicit model' } });
  expect(view.save().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(initialRoute);
  expect(resolve).not.toHaveBeenCalled();
});

test('failed resolution preserves the previous default and shows a product error', async () => {
  spyOn(agentPlatform.resolveChatModelRoute, 'invoke').mockRejectedValue(new Error('private host route details'));
  const view = mount();
  await view.choose('Second provider / Next model');
  await waitFor(() => expect(view.baseElement.textContent).toContain(en.fields.defaultModelFailed));
  expect(view.state().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(initialRoute);
  expect(view.state().document.model_route_refs[AGENT_CHAT_MODEL_TASK]).toBe(initialRoute.primary.model_route_id);
  expect(view.picker().textContent).toContain('Old model');
  expect(view.baseElement.textContent).not.toContain('private host route details');
});

test('clearing a pending choice unlocks saving and its late response cannot restore a default model', async () => {
  let finish!: (record: ChatRouteRecord) => void;
  const resolve = spyOn(agentPlatform.resolveChatModelRoute, 'invoke').mockImplementation(() =>
    new Promise<ChatRouteRecord>(resolve => { finish = resolve; })
  );
  const view = mount();
  const start = view.getByRole('button', { name: en.actions.startConversation }) as HTMLButtonElement;
  expect(start.disabled).toBe(false);
  await view.choose('Second provider / Next model');
  await waitFor(() => expect(resolve).toHaveBeenCalledTimes(1));
  expect(start.disabled).toBe(true);
  fireEvent.change(view.getByRole('textbox', { name: en.fields.persona }), { target: { value: 'Still keep this persona' } });
  const save = view.getByRole('button', { name: common.save }) as HTMLButtonElement;
  expect(save.disabled).toBe(true);
  await view.choose(en.fields.defaultModelPlaceholder);
  const saved = view.save();
  expect(saved.document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toBeUndefined();
  expect(saved.document.model_route_refs).toEqual({ unrelated_task: 'unrelated-route' });
  expect(saved.document.persona).toBe('Still keep this persona');
  await act(async () => finish(route(providerBId, 'next-model')));
  expect(view.state().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toBeUndefined();
  expect(view.state().document.model_route_refs).toEqual({ unrelated_task: 'unrelated-route' });
  expect(view.picker().textContent).toContain(en.fields.defaultModelPlaceholder);
  expect((view.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(false);
});

test('a pending default model choice completes while another editor tab is open', async () => {
  let finish!: (record: ChatRouteRecord) => void;
  const resolve = spyOn(agentPlatform.resolveChatModelRoute, 'invoke').mockImplementation(() =>
    new Promise<ChatRouteRecord>(resolve => { finish = resolve; })
  );
  const view = mount();
  await view.choose('Second provider / Next model');
  await waitFor(() => expect(resolve).toHaveBeenCalledTimes(1));
  fireEvent.click(view.getByRole('tab', { name: en.workbench.capabilityTab }));
  expect((view.getByRole('button', { name: en.actions.startConversation }) as HTMLButtonElement).disabled).toBe(true);
  const resolved = route(providerBId, 'next-model', 17);
  await act(async () => finish(resolved));
  await waitFor(() => expect(view.state().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(resolved));
  expect(view.save().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(resolved);
  fireEvent.click(view.getByRole('tab', { name: en.workbench.settingsTab }));
  expect(view.picker().textContent).toContain('Second provider / Next model');
});

test('a late response from an earlier model choice cannot replace the newer choice', async () => {
  let finishFirst!: (record: ChatRouteRecord) => void;
  let finishLatest!: (record: ChatRouteRecord) => void;
  const resolve = spyOn(agentPlatform.resolveChatModelRoute, 'invoke').mockImplementation(({ model }) =>
    new Promise<ChatRouteRecord>(resolve => {
      if (model.model === 'next-model') finishFirst = resolve;
      else finishLatest = resolve;
    })
  );
  const view = mount();
  await view.choose('Second provider / Next model');
  await waitFor(() => expect(resolve).toHaveBeenCalledTimes(1));
  await view.choose('Second provider / Latest model');
  await waitFor(() => expect(resolve).toHaveBeenCalledTimes(2));
  fireEvent.change(view.getByRole('textbox', { name: en.fields.persona }), { target: { value: 'Keep the edited persona' } });
  fireEvent.change(view.getByRole('textbox', { name: en.fields.instructions }), { target: { value: 'Keep the edited instructions' } });
  const latest = route(providerBId, 'latest-model', 23);
  await act(async () => finishLatest(latest));
  await waitFor(() => expect(view.state().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(latest));
  await act(async () => finishFirst(route(providerBId, 'next-model', 22)));
  expect(view.state().document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(latest);
  expect(view.picker().textContent).toContain('Latest model');
  const saved = view.save();
  expect(saved.document.chat_route_records[AGENT_CHAT_MODEL_TASK]).toEqual(latest);
  expect(saved.document.persona).toBe('Keep the edited persona');
  expect(saved.document.instructions).toBe('Keep the edited instructions');
});
