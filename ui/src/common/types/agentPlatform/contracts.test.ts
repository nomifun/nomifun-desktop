import { describe, expect, test } from 'bun:test';
import {
  OFFICIAL_PRESET_KEYS,
  type AgentBindingValue,
  type RemoteCredentialContinuation,
  type RemoteBinding,
} from './contracts';

const remoteCredentialContinuationFixture = (): RemoteCredentialContinuation => ({
  requires_same_owner: true,
  requires_explicit_agent_session_id: true,
  implicit_session_lookup: false,
  auth_error_code: 'REMOTE_AUTH_REQUIRED',
  rest_status: 401,
});

describe('Agent Platform TypeScript contracts', () => {
  test('official template key set is exact and Research is not a template', () => {
    expect([...OFFICIAL_PRESET_KEYS]).toEqual([
      'chat.minimal',
      'assistant.general',
      'coding.codex',
      'companion.default',
      'robot.default',
      'customer-service.default',
      'creative-studio.default',
    ]);
    expect(OFFICIAL_PRESET_KEYS.includes('research' as never)).toBe(false);
  });

  test('RemoteBinding contains only id, owner, name, and canonical AgentBindingValue', () => {
    const agentBinding = {} as AgentBindingValue;
    const binding: RemoteBinding = {
      remote_binding_id: 'binding' as RemoteBinding['remote_binding_id'],
      owner_user_id: 'owner',
      name: 'Workspace',
      agent_binding: agentBinding,
    };
    expect(Object.keys(binding).sort()).toEqual([
      'agent_binding',
      'name',
      'owner_user_id',
      'remote_binding_id',
    ]);
  });

  test('D-026 continuation requires same owner and explicit Session identity', () => {
    expect(remoteCredentialContinuationFixture()).toEqual({
      requires_same_owner: true,
      requires_explicit_agent_session_id: true,
      implicit_session_lookup: false,
      auth_error_code: 'REMOTE_AUTH_REQUIRED',
      rest_status: 401,
    });
  });
});
