/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { buildAgentConversationParams } from './buildAgentConversationParams';
import type { TProviderWithModel } from '@/common/config/storage';

const model: TProviderWithModel = {
  id: 'provider-1',
  name: 'Provider 1',
  platform: 'openai',
  base_url: 'https://example.invalid',
  api_key: '',
  use_model: 'model-1',
};

describe('buildAgentConversationParams preset contract', () => {
  test('sends only the preset reference at the top level for a preset launch', () => {
    const result = buildAgentConversationParams({
      backend: 'claude',
      name: 'Preset launch',
      agent_id: 'agent-1',
      agent_name: 'Claude',
      preset_id: 'preset-1',
      workspace: '/tmp/workspace',
      model,
      is_preset: true,
    });

    expect(result.preset_id).toBe('preset-1');
    expect('preset_id' in result.extra).toBe(false);
    expect('agent_id' in result.extra).toBe(false);
    expect('agent_name' in result.extra).toBe(false);
    expect('backend' in result.extra).toBe(false);
  });

  test('keeps bare Agent runtime identity and omits preset lineage', () => {
    const result = buildAgentConversationParams({
      backend: 'claude',
      name: 'Bare Agent launch',
      agent_id: 'agent-1',
      agent_name: 'Claude',
      workspace: '/tmp/workspace',
      model,
    });

    expect(result.preset_id).toBeUndefined();
    expect(result.extra.agent_id).toBe('agent-1');
    expect(result.extra.agent_name).toBe('Claude');
    expect(result.extra.backend).toBe('claude');
  });

  test('stores the selected remote-agent row id in snake_case', () => {
    const result = buildAgentConversationParams({
      backend: 'remote',
      name: 'Remote OpenClaw',
      workspace: '/tmp/workspace',
      model,
      custom_agent_id: '42',
    });

    expect(result.type).toBe('remote');
    expect(result.extra.remote_agent_id).toBe(42);
  });

  test('rejects a missing remote-agent row id', () => {
    let error: unknown;
    try {
      buildAgentConversationParams({
        backend: 'remote',
        name: 'Remote OpenClaw',
        workspace: '/tmp/workspace',
        model,
      });
    } catch (caught) {
      error = caught;
    }
    expect(error instanceof Error).toBe(true);
    expect((error as Error).message.includes('remote_agent_id')).toBe(true);
  });
});
