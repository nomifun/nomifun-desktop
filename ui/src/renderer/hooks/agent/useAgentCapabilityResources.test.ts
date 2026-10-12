import { describe, expect, test } from 'bun:test';
import {
  requiredResourceKindsForCapabilitySelections,
} from './useAgentCapabilityResources';
import {
  asCapabilityId,
  asPackageId,
  type CapabilityModuleCatalogItem,
} from '@/common/types/agentPlatform';

const capability = (
  id: string,
  required_resource_kinds: string[]
): CapabilityModuleCatalogItem => ({
  module: { id: asCapabilityId(id) },
  display_name: id,
  description: id,
  source_package: {
    id: asPackageId('test.package'),
    version: '1.0.0',
  },
  authoring_policy: 'direct',
  summary_kind: 'tool',
  actions: [{ action_id: `${id}/read`, input_schema: 'input', output_schema: 'output', effect_class: 'read_local', presentation: 'visible' }],
  context_schema_refs: [],
  event_schema_refs: [],
  required_host_ports: [],
  supported_surfaces: ['desktop'],
  required_resource_kinds,
  required_modules: [],
  conflicting_modules: [],
});

const catalog = [
  capability('knowledge/search', ['knowledge_base']),
  capability('workspace.bind', ['workspace']),
];

describe('Agent capability resource projection', () => {
  test('matches stable capability references by ID', () => {
    expect(
      [...requiredResourceKindsForCapabilitySelections(
        [{ capability: { id: asCapabilityId('knowledge/search') }, action_allowlist: ['knowledge/search/read'] }],
        catalog
      )]
    ).toEqual(['knowledge_base']);
  });

  test('returns no resource kind for an absent capability', () => {
    expect(
      requiredResourceKindsForCapabilitySelections(
        [{ capability: { id: asCapabilityId('missing.capability') }, action_allowlist: ['missing.capability/read'] }],
        catalog
      )
    ).toEqual(new Set());
  });

  const workshop: CapabilityModuleCatalogItem = {
    ...capability('creative.workshop', ['asset_library', 'canvas']),
    actions: [
      { action_id: 'creative.workshop/asset.read', input_schema: 'input', output_schema: 'output', effect_class: 'read_local', presentation: 'visible', required_resource_operations: { asset_library: ['read'] } },
      { action_id: 'creative.workshop/asset.write', input_schema: 'input', output_schema: 'output', effect_class: 'write_local', presentation: 'visible', required_resource_operations: { asset_library: ['write'] } },
      { action_id: 'creative.workshop/canvas.read', input_schema: 'input', output_schema: 'output', effect_class: 'read_local', presentation: 'visible', required_resource_operations: { canvas: ['read'] } },
    ],
  };

  test('asset-only grants do not ask for a canvas', () => {
    expect(requiredResourceKindsForCapabilitySelections([{
      capability: workshop.module,
      action_allowlist: ['creative.workshop/asset.read', 'creative.workshop/asset.write'],
    }], [workshop])).toEqual(new Set(['asset_library']));
  });

  test('an explicitly granted canvas action adds the canvas resource', () => {
    expect(requiredResourceKindsForCapabilitySelections([{
      capability: workshop.module,
      action_allowlist: ['creative.workshop/asset.read', 'creative.workshop/canvas.read'],
    }], [workshop])).toEqual(new Set(['asset_library', 'canvas']));
  });

  test('an explicit empty action contract consumes no resources', () => {
    const neutral = { ...workshop, actions: workshop.actions.map((action, index) =>
      index === 0 ? { ...action, required_resource_operations: {} } : action
    ) };
    expect(requiredResourceKindsForCapabilitySelections([{
      capability: workshop.module, action_allowlist: ['creative.workshop/asset.read'],
    }], [neutral])).toEqual(new Set());
  });

  test('missing or null action metadata retains the full module gate', () => {
    for (const requirements of [undefined, null]) {
      const legacy = { ...workshop, actions: [{ ...workshop.actions[0], required_resource_operations: requirements }] };
      expect(requiredResourceKindsForCapabilitySelections([{
        capability: workshop.module, action_allowlist: ['creative.workshop/asset.read'],
      }], [legacy])).toEqual(new Set(['asset_library', 'canvas']));
    }
  });

  test('context and event consumers retain their module resources', () => {
    for (const contribution of [{ context_schema_refs: ['context'] }, { event_schema_refs: ['event'] }]) {
      expect(requiredResourceKindsForCapabilitySelections([{
        capability: workshop.module, action_allowlist: ['creative.workshop/asset.read'],
      }], [{ ...workshop, ...contribution }])).toEqual(new Set(['asset_library', 'canvas']));
    }
  });

  test('non-action module resources remain required', () => {
    expect(requiredResourceKindsForCapabilitySelections([{
      capability: workshop.module, action_allowlist: ['creative.workshop/asset.read'],
    }], [{ ...workshop, required_resource_kinds: ['asset_library', 'canvas', 'knowledge_base'] }]))
      .toEqual(new Set(['knowledge_base', 'asset_library']));
  });
});
