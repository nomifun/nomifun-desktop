import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { useState } from 'react';
import { AGENT_PRESET_LIBRARY_SWR_KEY } from '@/renderer/hooks/agent/useAgentPresets';
import { useGuidAgentSelection } from '../guid/hooks/useGuidAgentSelection';
import {
  asAgentPresetId, asCapabilityId, asDigestHex, asPackageId,
  createEmptyAgentPresetDocument,
  type AgentPresetEditorResponse, type CapabilityCatalogItem,
  type CreateAgentPresetRequest, type OfficialPresetTemplate, type SaveAgentPresetRevisionRequest,
} from '@/common/types/agentPlatform';
import AgentPresetEditor from './AgentPresetEditor';
import OfficialTemplateOverview from './OfficialTemplateOverview';
import { runtimeEngineOptions } from './AgentRuntimeEngineSelector';
import { useAgentSettingsController } from './useAgentSettingsController';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import common from '../../services/i18n/locales/en-US/common.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en, common } } } });
const realFetch = globalThis.fetch;
afterEach(() => { cleanup(); globalThis.fetch = realFetch; });

for (const family of ['nomi', 'coding']) {
  test(`official template ${family} engine survives HTTP creation and reopening as a launchable personal Agent`, async () => {
    const presetId = asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000002');
    const reference = { preset_id: presetId, revision: 1, revision_digest: asDigestHex('a'.repeat(64)) };
    const template: OfficialPresetTemplate = {
      template_key: 'chat.minimal', immutable: true, forkable: true,
      seed: { enabled_capabilities: [], skill_bindings: [], required_resource_kinds: [], required_runtime_features: [] },
      role_coverage: { required_capability_categories: [], required_capability_ids: [], required_runtime_features: [], required_resource_kinds: [] },
    };
    const descriptor = {
      family_id: `nomifun.${family}`, build_id: 'bundled-test', build_digest: 'c'.repeat(64),
      host_contract_version: 1, display_name: `${family} Engine`, supported_profiles: ['workflow'],
    };
    const requests: CreateAgentPresetRequest[] = [];
    const launches: string[] = [];
    let editor: AgentPresetEditorResponse | undefined;
    // The transport is real; persistence and admission are a stubbed server.
    globalThis.fetch = (async (input, init) => {
      const path = new URL(String(input), 'http://127.0.0.1').pathname;
      let data: unknown;
      if (path === '/api/agent-preset-templates') data = {
        official_templates: [template], user_presets: editor ? [editor.preset] : [], active_bindings: [],
        fresh_start: { data_generation: 4, legacy_data_imported: false, official_template_count: 1, user_preset_count: editor ? 1 : 0 },
      };
      else if (path === '/api/agent-catalog') data = { capabilities: [], skills: [], mcp_tools: [], roles: [] };
      else if (path === '/api/agent-presets' && init?.method === 'POST') {
        const request = JSON.parse(String(init.body)) as CreateAgentPresetRequest;
        requests.push(request);
        const document = request.document!;
        editor = {
          preset: { preset_id: presetId, source: 'user', display_name: request.display_name, bound_target_count: 0, current_stable_revision: reference },
          revision: { reference, document, created_by: 'owner', created_at_ms: 1 },
          draft: { preset_id: presetId, display_name: request.display_name, current_revision: reference, document },
        };
        data = editor;
      } else if (path === `/api/agent-presets/${presetId}/editor`) data = editor;
      else if (path === '/api/settings/client') data = {};
      else throw new Error(`Unexpected test request: ${path}`);
      return new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });
    }) as typeof fetch;

    let controller!: ReturnType<typeof useAgentSettingsController>;
    const GuidProbe = () => {
      const target = useGuidAgentSelection({ selectedAgentPresetId: presetId, locationKey: `launch-${family}` });
      return <output data-testid='guid-selected-preset'>{target.selectedPreset?.preset_id ?? target.selection.kind}</output>;
    };
    const Harness = () => {
      controller = useAgentSettingsController();
      const [launched, setLaunched] = useState(false);
      if (launched) return <GuidProbe />;
      if (controller.editor && controller.draft) return <AgentPresetEditor
        editor={controller.editor} draft={controller.draft} catalog={controller.catalog}
        busyAction={controller.busyAction} dirty={controller.dirty}
        onDraftChange={controller.setDraft} onSave={() => { void controller.saveRevision(); }}
        onDiscard={controller.discardChanges} onStartConversation={(preset) => { launches.push(preset.preset_id); setLaunched(true); }}
      />;
      return <OfficialTemplateOverview template={template} busy={controller.busyAction !== null} catalog={controller.catalog}
        onSave={(name, document, description) => { void controller.createConfiguredPreset(name, document, description); }}
      />;
    };
    const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{
      provider: () => new Map(), revalidateOnMount: false,
      fallback: { 'runtime-engines': [descriptor], providers: [],
        [AGENT_PRESET_LIBRARY_SWR_KEY]: { official_templates: [template], user_presets: [], active_bindings: [] } },
    }}><Harness /></SWRConfig></MemoryRouter></I18nextProvider>);
    await waitFor(() => expect(controller.loading).toBe(false));
    expect(controller.error).toBeNull();
    fireEvent.click(screen.getByRole('tab', { name: en.workbench.settingsTab }));
    const option = runtimeEngineOptions([descriptor])[0];
    fireEvent.click(screen.getByRole('combobox', { name: en.runtimeEngine.label }));
    fireEvent.click(await screen.findByText(option.label));
    fireEvent.click(screen.getByRole('button', { name: en.workbench.saveAsMine }));
    await waitFor(() => { expect(requests).toHaveLength(1); expect(controller.busyAction).toBeNull(); });
    expect(controller.error).toBeNull();
    expect(requests[0].document?.runtime_engine).toEqual(option.selection);
    expect(controller.draft?.document.runtime_engine).toEqual(option.selection);
    expect(controller.dirty).toBe(false);

    await act(async () => { await controller.openPreset(editor!.preset); });
    fireEvent.click(screen.getByRole('tab', { name: en.workbench.settingsTab }));
    expect(screen.getByText(option.label)).toBeTruthy();
    expect(controller.draft?.current_revision).toEqual(reference);
    fireEvent.click(screen.getByRole('button', { name: en.actions.startConversation }));
    expect(launches).toEqual([presetId]);
    await waitFor(() => expect(screen.getByTestId('guid-selected-preset').textContent).toBe(presetId));
    expect(requests).toHaveLength(1);
  });
}

test('workbench engine and enabled capabilities survive revision save, reopen and reset to default', async () => {
  const presetId = asAgentPresetId('0190f5fe-7c00-7a00-8000-000000000001');
  const reference = { preset_id: presetId, revision: 1, revision_digest: asDigestHex('a'.repeat(64)) };
  const document = createEmptyAgentPresetDocument();
  document.chat_route_records.agent_chat = {
    schema: 'nomifun.chat-route-record.v1', task: 'agent_chat', failovers: [],
    primary: {
      model_route_id: 'route', model_route_revision: 1, provider_id: 'provider', model: 'model',
      protocol: 'openai_chat', connection_config_ref: 'default', credential_ref: 'credential',
      config_revision_digest: asDigestHex('b'.repeat(64)), features: ['text_input', 'text_output'],
    },
  };
  document.model_route_refs.agent_chat = 'route';
  const capability: CapabilityCatalogItem = {
    capability: { id: asCapabilityId('fs.read'), version: '1.0.0' }, kind: 'tool',
    display_name: 'Read files', description: 'Read workspace files',
    source_package: { id: asPackageId('nomifun.files'), version: '1.0.0' },
    source_kind: 'bundled', materialization_state: 'materialized', supported_surfaces: ['desktop'],
    required_runtime_features: [], required_resource_kinds: [], required_capabilities: [],
    conflicting_capabilities: [], action_count: 1, context_contributor_count: 0,
  };
  let editor: AgentPresetEditorResponse = {
    preset: { preset_id: presetId, source: 'user', display_name: 'Windows Agent', bound_target_count: 0, current_stable_revision: reference },
    revision: { reference, document, created_by: 'owner', created_at_ms: 1 },
    draft: { preset_id: presetId, display_name: 'Windows Agent', current_revision: reference, document },
  };
  const descriptor = {
    family_id: 'nomifun.coding', build_id: 'windows-test', build_digest: 'c'.repeat(64),
    host_contract_version: 1, display_name: 'Coding Engine', supported_profiles: ['workflow'],
  };
  const saves: SaveAgentPresetRevisionRequest[] = [];
  // Exercise the real HTTP bridge without any service, model or engine process.
  globalThis.fetch = (async (input, init) => {
    const path = new URL(String(input), 'http://127.0.0.1').pathname;
    let data: unknown;
    if (path === '/api/agent-preset-templates') data = {
      official_templates: [], user_presets: [editor.preset], active_bindings: [],
      fresh_start: { data_generation: 4, legacy_data_imported: false, official_template_count: 0, user_preset_count: 1 },
    };
    else if (path === '/api/agent-catalog') data = { capabilities: [capability], skills: [], mcp_tools: [], roles: [] };
    else if (path === `/api/agent-presets/${presetId}/editor`) data = editor;
    else if (path === `/api/agent-presets/${presetId}/revisions` && init?.method === 'POST') {
      const request = JSON.parse(String(init.body)) as SaveAgentPresetRevisionRequest;
      saves.push(request);
      const next = { ...reference, revision: saves.length + 1, revision_digest: asDigestHex('d'.repeat(64)) };
      editor = {
        preset: { ...editor.preset, current_stable_revision: next },
        revision: { reference: next, document: request.draft.document, created_by: 'owner', created_at_ms: 2 },
        draft: { ...request.draft, current_revision: next },
      };
      data = { preset: editor.preset, revision: editor.revision, resolved_snapshot_ref: { snapshot_id: 'snapshot', snapshot_digest: 'e'.repeat(64) } };
    } else throw new Error(`Unexpected test request: ${path}`);
    return new Response(JSON.stringify({ success: true, data }), { headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;

  let controller!: ReturnType<typeof useAgentSettingsController>;
  const Harness = () => {
    controller = useAgentSettingsController();
    return controller.editor && controller.draft ? <AgentPresetEditor
      editor={controller.editor} draft={controller.draft} catalog={controller.catalog}
      busyAction={controller.busyAction} dirty={controller.dirty}
      onDraftChange={controller.setDraft} onSave={() => { void controller.saveRevision(); }}
      onDiscard={controller.discardChanges} onStartConversation={() => {}}
    /> : null;
  };
  const screen = render(<I18nextProvider i18n={i18n}><MemoryRouter><SWRConfig value={{
    provider: () => new Map(), revalidateOnMount: false,
    fallback: { 'runtime-engines': [descriptor], providers: [] },
  }}><Harness /></SWRConfig></MemoryRouter></I18nextProvider>);
  await waitFor(() => expect(controller.loading).toBe(false));
  expect(controller.error).toBeNull();
  await act(async () => { await controller.openPreset(editor.preset); });
  fireEvent.click(screen.getByRole('tab', { name: en.workbench.settingsTab }));
  fireEvent.click(screen.getByRole('combobox', { name: en.runtimeEngine.label }));
  fireEvent.click(await screen.findByText('Coding Engine · workflow · windows-test'));
  await waitFor(() => expect(controller.dirty).toBe(true));
  fireEvent.click(screen.getByRole('tab', { name: en.workbench.capabilityTab }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Add Read files' }));
  fireEvent.click(screen.getByRole('button', { name: 'Move in (1)' }));
  fireEvent.click(screen.getByRole('button', { name: common.save }));
  await waitFor(() => { expect(saves).toHaveLength(1); expect(controller.busyAction).toBeNull(); });
  expect(saves[0].expected_current_revision).toEqual(reference);
  expect(saves[0].draft.document.runtime_engine).toEqual(runtimeEngineOptions([descriptor])[0].selection);
  expect(saves[0].draft.document.enabled_capabilities.map((entry) => entry.capability)).toEqual([capability.capability]);
  expect(saves[0].draft.document).not.toHaveProperty('initial_capabilities');
  expect(saves[0].draft.document).not.toHaveProperty('on_demand_capabilities');
  expect(controller.dirty).toBe(false);
  await act(async () => { await controller.openPreset(editor.preset); });
  expect(controller.draft?.document.runtime_engine).toEqual(saves[0].draft.document.runtime_engine);
  fireEvent.click(screen.getByRole('tab', { name: en.workbench.settingsTab }));
  fireEvent.click(screen.getByRole('combobox', { name: en.runtimeEngine.label }));
  fireEvent.click(await screen.findByText(en.runtimeEngine.default));
  fireEvent.click(screen.getByRole('button', { name: common.save }));
  await waitFor(() => { expect(saves).toHaveLength(2); expect(controller.busyAction).toBeNull(); });
  expect(saves[1].expected_current_revision?.revision).toBe(2);
  expect(saves[1].draft.document).not.toHaveProperty('runtime_engine');
  expect(saves[1].draft.document.enabled_capabilities).toEqual(saves[0].draft.document.enabled_capabilities);
  expect(controller.dirty).toBe(false);
});

test('conversation and Guid do not author or override the Agent engine', () => {
  for (const path of ['../conversation/platforms/nomi/NomiSendBox.tsx', '../guid/GuidPage.tsx']) {
    const source = readFileSync(new URL(path, import.meta.url), 'utf8');
    expect(source).not.toMatch(/AgentRuntimeEngineSelector|runtime_engine|runtimeEngines/);
    expect(source).not.toMatch(/initial_capabilities|on_demand_capabilities/);
  }
});
