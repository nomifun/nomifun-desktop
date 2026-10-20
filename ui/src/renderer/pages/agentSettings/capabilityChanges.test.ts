import { expect, test } from 'bun:test';
import {
  asCapabilityId,
  asPackageId,
  createEmptyAgentPresetDocument,
  type AgentCatalogResponse,
  type CapabilityModuleCatalogItem,
} from '@/common/types/agentPlatform';
import { planModuleChange, setModuleActions } from './capabilityChanges';
import computer from '../../../../../crates/backend/nomifun-agent-contracts/contracts/computer-tools.v1.json';

const moduleItem = (
  id: string,
  required: CapabilityModuleCatalogItem[] = [],
  authoring_policy: CapabilityModuleCatalogItem['authoring_policy'] = 'direct',
): CapabilityModuleCatalogItem => ({
  module: { id: asCapabilityId(id) }, display_name: id, description: id,
  source_package: { id: asPackageId('nomifun.test'), version: '1.0.0' }, authoring_policy, summary_kind: 'tool',
  actions: [{ action_id: `${id}/run`, effect_class: 'pure', presentation: 'function_tool' }],
  context_schema_refs: [], event_schema_refs: [], required_resource_kinds: [], required_host_ports: [],
  required_modules: required.map((item) => item.module), conflicting_modules: [], supported_surfaces: ['desktop'],
});
const read = moduleItem('workspace.read', [], 'dependency_only');
const write = moduleItem('workspace.write', [read], 'dependency_only');
const work = moduleItem('workspace.edit', [write]);
work.actions.push({ action_id: 'workspace.edit/delete', effect_class: 'destructive', presentation: 'function_tool' });
const modules = [read, write, work];
const catalog = (items = modules): AgentCatalogResponse => ({
  modules: items,
  capabilities: items.map((item) => ({
    capability: item.module, kind: 'tool', display_name: item.display_name, description: item.description,
    source_package: item.source_package, source_kind: 'bundled', materialization_state: 'materialized', supported_surfaces: ['desktop'],
    required_runtime_features: [], required_resource_kinds: [], required_capabilities: item.required_modules,
    conflicting_capabilities: item.conflicting_modules, action_count: item.actions.length, context_contributor_count: 0,
  })),
  skills: [], mcp_tools: [], roles: [],
});
const empty = createEmptyAgentPresetDocument();

test('an enable plan validates dependency closure without authoring dependency roots or risky actions', () => {
  const plan = planModuleChange(empty, catalog(), [work.module], true);
  expect(plan.additional).toEqual([]);
  expect(plan.document.enabled_capabilities).toEqual([{
    capability: work.module,
    action_allowlist: ['workspace.edit/run'],
  }]);
});

test('enabling Computer selects its entire frozen action set without changing other Module defaults', () => {
  const module = moduleItem('computer');
  module.actions = computer.tools.map((tool) => ({
    action_id: tool.action_id, effect_class: tool.effect_class, presentation: 'function_tool',
  }));
  const plan = planModuleChange(empty, catalog([...modules, module]), [module.module, work.module], true);
  expect(plan.blocked).toEqual([]);
  expect(plan.document.enabled_capabilities).toEqual([{
    capability: module.module,
    action_allowlist: computer.tools.map((tool) => tool.action_id).sort((left, right) => left.localeCompare(right)),
  }, {
    capability: work.module,
    action_allowlist: ['workspace.edit/run'],
  }]);
  expect(plan.document.enabled_capabilities[0].action_allowlist).toHaveLength(22);
});

test('enabling an already selected Computer preserves the explicit action selection', () => {
  const module = moduleItem('computer');
  module.actions = computer.tools.map((tool) => ({
    action_id: tool.action_id, effect_class: tool.effect_class, presentation: 'function_tool',
  }));
  for (const action_allowlist of [[], [computer.tools[0].action_id]]) {
    const document = {
      ...empty,
      enabled_capabilities: [{ capability: module.module, action_allowlist }],
    };
    expect(planModuleChange(document, catalog([module]), [module.module], true).document).toBe(document);
  }
});

test('a missing dependency blocks the whole Module edit', () => {
  const plan = planModuleChange(empty, catalog([write, work]), [work.module], true);
  expect(plan.blocked).toEqual([read.module]);
  expect(plan.document).toBe(empty);
});

test('disabling a direct Module does not mutate Compiler-owned dependencies', () => {
  const document = {
    ...empty,
    enabled_capabilities: [{ capability: work.module, action_allowlist: ['workspace.edit/run'] }],
  };
  const plan = planModuleChange(document, catalog(), [work.module], false);
  expect(plan.additional).toEqual([]);
  expect(plan.document.enabled_capabilities).toEqual([]);
});

test('action edits preserve the exact Module identity', () => {
  const document = {
    ...empty,
    enabled_capabilities: [{ capability: read.module, action_allowlist: ['workspace.read/run'] }],
  };
  const changed = setModuleActions(document, read.module, ['workspace.read/inspect']);
  expect(changed.enabled_capabilities).toEqual([{
    capability: read.module,
    action_allowlist: ['workspace.read/inspect'],
  }]);
});
