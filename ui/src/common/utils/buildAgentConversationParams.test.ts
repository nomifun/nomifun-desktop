/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { buildAgentConversationParams } from './buildAgentConversationParams';
import type { TProviderWithModel } from '@/common/config/storage';
import { parseAgentId, parseProviderId } from '@/common/types/ids';

const agentId = parseAgentId('0190f5fe-7c00-7a00-8000-000000000002');

const model: TProviderWithModel = {
  id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000001'),
  name: 'Provider 1',
  platform: 'openai',
  base_url: 'https://example.invalid',
  auth_scheme: 'bearer',
  has_credentials: false,
  use_model: 'model-1',
};

describe('buildAgentConversationParams', () => {
  test('keeps the ordinary Agent runtime identity in the request extra', () => {
    const result = buildAgentConversationParams({
      backend: 'claude',
      name: 'Agent launch',
      agent_id: agentId,
      agent_name: 'Claude',
      workspace: '/tmp/workspace',
      model,
    });

    expect(result.preset_id).toBeUndefined();
    expect(result.extra.agent_id).toBe(agentId);
    expect(result.extra.agent_name).toBe('Claude');
    expect(result.extra.backend).toBe('claude');
  });
});
