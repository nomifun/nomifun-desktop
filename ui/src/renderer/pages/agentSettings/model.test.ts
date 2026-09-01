import { describe, expect, test } from 'bun:test';
import type { AgentPresetDraft } from '@/common/types/agentPlatform';
import {
  DEFAULT_WORKSPACE_RESOURCE_ID,
  WORKSPACE_ROOT_PARAMETER,
  withHostResolvedWorkspaceBinding,
} from './model';

const draft = (): AgentPresetDraft => ({
  preset_id: '0190f5fe-7c00-7a00-8000-000000000001' as AgentPresetDraft['preset_id'],
  display_name: 'Coding',
  document: {
    schema_version: '1.0.0',
    surfaces: ['desktop'],
    model_route_refs: {},
    chat_route_records: {},
    initial_capabilities: [],
    on_demand_capabilities: [],
    skill_bindings: [],
    resource_bindings: [
      {
        binding_id: 'workspace',
        resource_kind: 'workspace',
        resource_id: '',
        owner_id: 'owner',
        operations: ['read', 'write'],
        typed_parameters: {},
      },
    ],
    persona: '',
    instructions: '',
    context_policy: {},
    execution_constraints: {},
    runtime_budget: {},
  },
});

describe('Agent Settings host resource resolution', () => {
  test('adds the host workspace path without treating resource_id as a path', () => {
    const resolved = withHostResolvedWorkspaceBinding(
      draft(),
      'C:\\work\\nomifun'
    );
    const binding = resolved.document.resource_bindings[0];

    expect(binding.resource_id).toBe(DEFAULT_WORKSPACE_RESOURCE_ID);
    expect(binding.typed_parameters?.[WORKSPACE_ROOT_PARAMETER]).toBe(
      'C:\\work\\nomifun'
    );
  });

  test('is stable once the exact host path has been resolved', () => {
    const first = withHostResolvedWorkspaceBinding(draft(), '/work/nomifun');
    const second = withHostResolvedWorkspaceBinding(first, '/work/nomifun');

    expect(second).toBe(first);
  });
});
