import { describe, expect, test } from 'bun:test';
import {
  requiredResourceKindsForCapabilityReferences,
} from './useAgentCapabilityResources';
import {
  asCapabilityId,
  asPackageId,
  type CapabilityCatalogItem,
} from '@/common/types/agentPlatform';

const capability = (
  id: string,
  version: string,
  required_resource_kinds: string[]
): CapabilityCatalogItem => ({
  capability: { id: asCapabilityId(id), version },
  kind: 'tool',
  display_name: id,
  description: id,
  source_package: {
    id: asPackageId('test.package'),
    version: '1.0.0',
  },
  source_kind: 'first_party',
  materialization_state: 'materialized',
  supported_surfaces: ['desktop'],
  required_runtime_features: [],
  required_resource_kinds,
  required_capabilities: [],
  conflicting_capabilities: [],
  action_count: 0,
  context_contributor_count: 0,
});

const catalog = [
  capability('knowledge.search', '1.0.0', ['knowledge_base']),
  capability('workspace.bind', '1.0.0', ['workspace']),
  capability('knowledge.search', '2.0.0', ['different_resource']),
];

describe('Agent capability resource projection', () => {
  test('matches exact capability references, including version', () => {
    expect(
      [...requiredResourceKindsForCapabilityReferences(
        [{ id: asCapabilityId('knowledge.search'), version: '1.0.0' }],
        catalog
      )]
    ).toEqual(['knowledge_base']);
  });

  test('returns no resource kind for an absent capability', () => {
    expect(
      requiredResourceKindsForCapabilityReferences(
        [{ id: asCapabilityId('missing.capability'), version: '1.0.0' }],
        catalog
      )
    ).toEqual(new Set());
  });
});
