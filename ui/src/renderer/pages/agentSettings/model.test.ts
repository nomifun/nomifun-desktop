import { describe, expect, test } from 'bun:test';
import type {
  AgentPresetDraft,
  CapabilityModuleCatalogItem,
  ChatRouteRecord,
} from '@/common/types/agentPlatform';
import { asCapabilityId, asPackageId } from '@/common/types/agentPlatform';
import { createDefaultIdmmConfig } from '@/common/types/idmm';
import {
  actionFallbackName,
  agentUiErrorKey,
  agentUiErrorSubjects,
  capabilityPlacement,
  classifyAgentUiError,
  moduleI18nKey,
  moduleMatchesSearch,
  placeCapability,
  selectChatRouteCandidate,
} from './model';

const draft = (): AgentPresetDraft => ({
  preset_id: '0190f5fe-7c00-7a00-8000-000000000001' as AgentPresetDraft['preset_id'],
  display_name: 'Coding',
  document: {
    schema_version: '1.0.0',
    model_route_refs: {},
    chat_route_records: {},
    enabled_capabilities: [],
    skill_bindings: [],
    system_role_provider_overrides: {},
    persona: '',
    instructions: '',
    starter_prompts: [],
    runtime_policy: { idmm: createDefaultIdmmConfig() },
  },
});

const moduleItem = (id: string): CapabilityModuleCatalogItem => ({
  module: { id: asCapabilityId(id) },
  display_name: 'Workspace files',
  description: 'Read and change workspace files',
  source_package: { id: asPackageId('nomifun.workspace'), version: '1.0.0' },
  authoring_policy: 'direct',
  summary_kind: 'tool',
  actions: [{
    action_id: `${id}/write`, input_schema: 'input', output_schema: 'output',
    effect_class: 'write_durable', presentation: 'function_tool',
  }],
  context_schema_refs: [], event_schema_refs: [], required_resource_kinds: ['workspace'],
  required_host_ports: [], required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
});

describe('Agent Settings Module authoring model', () => {
  test('maps only current Module identities to localized product copy', () => {
    expect(moduleI18nKey('browser')).toBe('browser');
    expect(moduleI18nKey('workspace.files')).toBe('workspaceFiles');
    expect(moduleI18nKey('workspace.files/read')).toBeUndefined();
    expect(moduleI18nKey('nomi_system_browser')).toBeUndefined();
  });

  test('searches canonical action and resource metadata supplied by the server', () => {
    const module = moduleItem('workspace.files');
    expect(moduleMatchesSearch(module, 'workspace.files/write', 'Workspace files', 'Files')).toBe(true);
    expect(moduleMatchesSearch(module, 'workspace', 'Workspace files', 'Files')).toBe(true);
    expect(moduleMatchesSearch(module, 'browser', 'Workspace files', 'Files')).toBe(false);
    expect(actionFallbackName('creative.workshop/canvas.read')).toBe('Canvas Read');
  });

  test('enabling is idempotent and disabling removes the exact Module', () => {
    const target = moduleItem('workspace.process').module;
    const enabled = placeCapability(draft().document, target, 'enabled');
    expect(capabilityPlacement(enabled, target)).toBe('enabled');
    expect(placeCapability(enabled, target, 'enabled').enabled_capabilities).toHaveLength(1);
    expect(placeCapability(enabled, target, 'none').enabled_capabilities).toEqual([]);
  });

  test('reorders an exact model route candidate without changing its contract', () => {
    const record = {
      schema: 'nomifun.chat-route-record.v1', task: 'agent_chat',
      primary: {
        model_route_id: 'route-primary', model_route_revision: 1, provider_id: 'provider-primary',
        model: 'model-primary', protocol: 'openai_chat', connection_config_ref: 'connection-primary',
        config_revision_digest: 'a'.repeat(64), credential_ref: 'credential-primary', features: ['text_input', 'text_output'],
      },
      failovers: [{
        model_route_id: 'route-fallback', model_route_revision: 2, provider_id: 'provider-fallback',
        model: 'model-fallback', protocol: 'openai_responses', connection_config_ref: 'connection-fallback',
        config_revision_digest: 'b'.repeat(64), credential_ref: 'credential-fallback', features: ['text_input', 'text_output'],
      }],
    } as ChatRouteRecord;
    const selected = selectChatRouteCandidate(record, 'route-fallback@2');
    expect(selected?.primary).toEqual(record.failovers[0]);
    expect(selected?.failovers).toEqual([record.primary]);
  });

  test('distinguishes a preset that is already absent', () => {
    expect(classifyAgentUiError({ code: 'AGENT_PRESET_NOT_FOUND', status: 404 }, 'delete')).toBe('preset-not-found');
  });

  test('keeps capability diagnostics distinct from model configuration errors', () => {
    const error = {
      code: 'CAPABILITY_NOT_MATERIALIZED',
      status: 422,
      details: { diagnostics: [
        { code: 'CAPABILITY_NOT_MATERIALIZED', subject: 'browser' },
        { code: 'CAPABILITY_UNAVAILABLE', subject: 'computer' },
      ] },
    };
    expect(classifyAgentUiError(error, 'save')).toBe('capability');
    expect(agentUiErrorKey(error, 'save')).toBe('agentSettings.errors.capability');
    expect(agentUiErrorSubjects(error)).toEqual(['browser', 'computer']);
  });
});
