import { expect, test } from 'bun:test';
import {
  asCapabilityId,
  asPackageId,
  createEmptyAgentPresetDocument,
  type AgentCatalogResponse,
  type CapabilityModuleCatalogItem,
} from '@/common/types/agentPlatform';
import {
  moduleCategory,
  requiredModuleReferences,
  unavailableModuleReferences,
} from './capabilityGroups';

const moduleItem = (id: string): CapabilityModuleCatalogItem => ({
  module: { id: asCapabilityId(id), version: '1.0.0' },
  display_name: id,
  description: id,
  source_package: { id: asPackageId('nomifun.test'), version: '1.0.0' },
  authoring_policy: 'direct',
  summary_kind: 'tool',
  actions: [{ action_id: `${id}/read`, input_schema: 'input', output_schema: 'output', effect_class: 'read_local', presentation: 'function_tool' }],
  context_schema_refs: [], event_schema_refs: [], required_resource_kinds: [], required_host_ports: [],
  required_modules: [], conflicting_modules: [], supported_surfaces: ['desktop'],
});

const availableCapability = (module: CapabilityModuleCatalogItem) => ({
  capability: module.module,
  kind: module.summary_kind,
  display_name: module.display_name,
  description: module.description,
  source_package: module.source_package,
  source_kind: 'bundled' as const,
  materialization_state: 'materialized' as const,
  supported_surfaces: ['desktop'],
  required_runtime_features: [],
  required_resource_kinds: [],
  required_capabilities: module.required_modules,
  conflicting_capabilities: [],
  action_count: module.actions.length,
  context_contributor_count: 0,
});

test('groups provider-neutral Browser and device Modules by product category', () => {
  expect(moduleCategory(moduleItem('browser').module)).toBe('web');
  expect(moduleCategory(moduleItem('web.research').module)).toBe('web');
  expect(moduleCategory(moduleItem('computer').module)).toBe('devices');
  expect(moduleCategory(moduleItem('robot').module)).toBe('devices');
  expect(moduleCategory(moduleItem('agent.tool-discovery').module)).toBe('integrations');
});

test('marks missing exact actions as unavailable without dropping the saved grant', () => {
  const module = moduleItem('workspace.files');
  const catalog: AgentCatalogResponse = {
    modules: [module],
    capabilities: [{
      capability: module.module, kind: 'tool', display_name: 'Files', description: '',
      source_package: module.source_package, source_kind: 'bundled', materialization_state: 'materialized',
      supported_surfaces: ['desktop'], required_runtime_features: [], required_resource_kinds: [],
      required_capabilities: [], conflicting_capabilities: [], action_count: 1, context_contributor_count: 0,
    }],
    skills: [], mcp_tools: [], roles: [],
  };
  const document = {
    ...createEmptyAgentPresetDocument(),
    enabled_capabilities: [{
      capability: module.module,
      action_allowlist: ['workspace.files/read', 'workspace.files/retired'],
    }],
  };
  expect(unavailableModuleReferences(document, catalog)).toEqual([module.module]);
  expect(document.enabled_capabilities[0].action_allowlist).toContain('workspace.files/retired');
});

test('projects transitive compiler dependencies without authoring them into the preset', () => {
  const leaf = { ...moduleItem('platform.guard'), authoring_policy: 'platform_managed' as const };
  const middle = {
    ...moduleItem('workspace.guard'),
    authoring_policy: 'dependency_only' as const,
    required_modules: [leaf.module],
  };
  const root = { ...moduleItem('workspace.files'), required_modules: [middle.module] };
  const document = {
    ...createEmptyAgentPresetDocument(),
    enabled_capabilities: [{
      capability: root.module,
      action_allowlist: ['workspace.files/read'],
    }],
  };

  expect(requiredModuleReferences(document, [root, middle, leaf])).toEqual([
    { module: leaf.module, requiredBy: [root.module] },
    { module: middle.module, requiredBy: [root.module] },
  ]);
  expect(unavailableModuleReferences(document, {
    modules: [root, middle, leaf],
    capabilities: [availableCapability(root), availableCapability(middle)],
  })).toEqual([leaf.module]);
  expect(document.enabled_capabilities).toHaveLength(1);
});
