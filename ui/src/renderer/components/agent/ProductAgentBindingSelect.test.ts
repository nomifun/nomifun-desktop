import { describe, expect, test } from 'bun:test';
import type {
  AgentPresetEditorResponse,
  OfficialPresetTemplate,
} from '@/common/types/agentPlatform';
import { templateForEditor } from './ProductAgentBindingSelect';

const template = {
  template_key: 'companion.default',
  seed: {
    enabled_capabilities: [
      { id: 'companion.persona', version: '1.0.0' },
      { id: 'memory.companion.recall', version: '1.0.0' },
    ],
    skill_bindings: [],
    required_resource_kinds: ['companion', 'companion_memory'],
    required_runtime_features: [],
  },
  role_coverage: {
    required_capability_categories: [],
    required_capability_ids: [],
    required_runtime_features: [],
    required_resource_kinds: ['companion', 'companion_memory'],
  },
  immutable: true,
  forkable: true,
} as unknown as OfficialPresetTemplate;

const editor = (capabilityIds: string[]) => ({
  revision: {
    document: {
      enabled_capabilities: capabilityIds.map((id) => ({ capability: { id, version: '1.0.0' } })),
      skill_bindings: [],
    },
  },
} as unknown as AgentPresetEditorResponse);

describe('product Agent binding identity', () => {
  test('recognizes an internal official configuration without relying on its hidden preset id', () => {
    expect(templateForEditor(editor([
      'memory.companion.recall',
      'companion.persona',
    ]), [template])?.template_key).toBe('companion.default');
    expect(templateForEditor(editor(['companion.persona']), [template])).toBeUndefined();
  });
});
