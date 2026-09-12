import { describe, expect, test } from 'bun:test';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import {
  agentSwitchRequiresWebSearchModel,
  classifyAgentSwitchError,
  resourceSelectionValueFromSessionExtra,
} from './agentSwitch';

const backendError = (code: string, details?: unknown) => new BackendHttpError({
  method: 'PUT',
  path: '/api/agent-sessions/session/preset',
  status: 422,
  body: { success: false, error: 'blocked', code, details },
});

describe('Agent switch failure classification', () => {
  test('keeps model, resource and capability failures actionable', () => {
    expect(classifyAgentSwitchError(backendError('MODEL_ROUTE_FEATURES_MISSING')))
      .toBe('model_incompatible');
    expect(classifyAgentSwitchError(backendError('RESOURCE_SELECTION_REQUIRED')))
      .toBe('resources_unavailable');
    expect(classifyAgentSwitchError(backendError('CAPABILITY_NOT_MATERIALIZED')))
      .toBe('capabilities_unavailable');
    expect(classifyAgentSwitchError(new Error('network'))).toBe('unknown');
  });

  test('recognizes the exact model requirement for native web search', () => {
    expect(agentSwitchRequiresWebSearchModel(backendError(
      'MODEL_ROUTE_FEATURES_MISSING',
      { missing_features: ['WebSearch'], required_protocol: 'openai.responses' },
    ))).toBe(true);
    expect(agentSwitchRequiresWebSearchModel(
      backendError('MODEL_ROUTE_FEATURES_MISSING', { missing_features: ['VisionInput'] }),
    )).toBe(false);
  });

  test('reads reusable user resources from the frozen Session binding', () => {
    expect(resourceSelectionValueFromSessionExtra({
      nomi_core_session: {
        binding: {
          typed_resource_bindings: [
            { resource_kind: 'knowledge_base', resource_id: 'knowledge-a' },
            { resource_kind: 'companion_memory', resource_id: 'companion-a' },
            { resource_kind: 'workspace', resource_id: 'default-workspace' },
          ],
        },
      },
    })).toEqual({ knowledge_base: 'knowledge-a', companion: 'companion-a' });
  });
});
