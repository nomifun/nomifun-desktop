/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import { describe, expect, test } from 'bun:test';
import { fromApiConversation } from '@/common/adapter/apiModelMapper';
import { parseProviderId } from '@/common/types/ids';
import { currentModelProviderTarget } from './currentModelProviderTarget';

const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000097');
const conversation = fromApiConversation({
  conversation_id: '0190f5fe-7c00-7a00-8000-000000000001', name: 'Gateway account action', type: 'nomi',
  created_at: 1, modified_at: 2, extra: {}, model: { provider_id: providerId, model: 'mock-compatible' },
});
describe('current account UI provider target', () => {
  test('resolves the actual wire model reference by exact provider identity instead of its empty platform stub', () => {
    expect(conversation.model.platform).toBe('');
    expect(currentModelProviderTarget(conversation.model, [{ id: providerId, platform: 'nomifun-model-gateway' }]))
      .toEqual({ id: providerId, platform: 'nomifun-model-gateway', use_model: 'mock-compatible' });
    expect(conversation.model.platform).toBe('');
  });
  test('does not borrow another gateway or infer a type when the selected provider is unresolved', () => {
    expect(currentModelProviderTarget(conversation.model, [{
      id: parseProviderId('0190f5fe-7c00-7a00-8000-000000000098'), platform: 'nomifun-model-gateway',
    }])).toBeUndefined();
    expect(currentModelProviderTarget(conversation.model, [])).toBeUndefined();
  });
  test('keeps known direct provider families distinct from the optional gateway', () => {
    expect(currentModelProviderTarget(conversation.model, [{ id: providerId, platform: 'openai' }])?.platform).toBe('openai');
  });
});
