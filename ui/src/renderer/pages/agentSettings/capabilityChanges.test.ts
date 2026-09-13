import { expect, test } from 'bun:test';
import { asCapabilityId, asPackageId, createEmptyAgentPresetDocument, type CapabilityCatalogItem } from '@/common/types/agentPlatform';
import { planCapabilityChange } from './capabilityChanges';

const capability = (id: string, required: CapabilityCatalogItem[] = []): CapabilityCatalogItem => ({
  capability: { id: asCapabilityId(id), version: '1.0.0' }, kind: 'tool', display_name: id, description: id,
  source_package: { id: asPackageId('test'), version: '1.0.0' }, source_kind: 'bundled', materialization_state: 'materialized',
  supported_surfaces: ['desktop'], required_runtime_features: [], required_resource_kinds: [],
  required_capabilities: required.map(item => item.capability), conflicting_capabilities: [], action_count: 1, context_contributor_count: 0,
});
const read = capability('fs.read'), write = capability('fs.write', [read]), work = capability('workspace.edit', [write]);
const catalog = [read, write, work];
const empty = createEmptyAgentPresetDocument();

test('an enable plan reports its full transitive dependencies before changing scope', () => {
  const plan = planCapabilityChange(empty, catalog, [work.capability], true);
  expect(plan.additional.map(item => item.id)).toEqual([read.capability.id, write.capability.id]);
  expect(plan.document.enabled_capabilities).toHaveLength(3);
  expect(empty.enabled_capabilities).toEqual([]);
});
test('a missing dependency blocks the entire batch', () => {
  const plan = planCapabilityChange(empty, [write, work], [work.capability], true);
  expect(plan.blocked).toEqual([read.capability]);
  expect(plan.document).toBe(empty);
});
test('conflicts are checked from both the existing and added capability', () => {
  const conflict = { ...read, conflicting_capabilities: [work.capability] };
  const doc = { ...empty, enabled_capabilities: [{ capability: read.capability }] };
  const plan = planCapabilityChange(doc, [conflict, write, work], [work.capability], true);
  expect(plan.blocked).toEqual([work.capability]);
  expect(plan.document).toBe(doc);
});
test('disabling a dependency includes transitive dependents, without touching other capabilities', () => {
  const other = capability('web.fetch');
  const doc = { ...empty, enabled_capabilities: [...catalog, other].map(item => ({ capability: item.capability })) };
  const plan = planCapabilityChange(doc, [...catalog, other], [read.capability], false);
  expect(plan.additional.map(item => item.id)).toEqual([write.capability.id, work.capability.id]);
  expect(plan.document.enabled_capabilities).toEqual([{ capability: other.capability }]);
});
test('dependency cycles cannot partially enable a batch', () => {
  const cycle = { ...read, required_capabilities: [work.capability] };
  const plan = planCapabilityChange(empty, [cycle, write, work], [work.capability], true);
  expect(plan.blocked.length).toBeGreaterThan(0);
  expect(plan.document).toBe(empty);
});
