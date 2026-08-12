/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IProvider } from '@/common/config/storage';
import { parseProviderId } from '@/common/types/ids';
import { exactChatHealthDotColor } from './guidModelHealth';

const MODEL = 'shared-model-id';
const PROVIDER_A = parseProviderId('0190f5fe-7c00-7a00-8000-000000000201');
const PROVIDER_B = parseProviderId('0190f5fe-7c00-7a00-8000-000000000202');

const provider = (
  id: IProvider['id'],
  health: 'healthy' | 'unhealthy',
  task: 'chat' | 'embedding' = 'chat'
): IProvider => ({
  id,
  platform: 'custom',
  name: id,
  base_url: 'https://example.invalid/v1',
  auth_scheme: 'bearer',
  has_credentials: true,
  enabled: true,
  models: [
    {
      provider_id: id,
      model: MODEL,
      enabled: true,
      sort_order: 0,
      capabilities: [
        {
          task,
          traits: [],
          protocol: task === 'chat' ? 'openai.chat_text' : 'openai.embeddings',
          connection_role: 'default',
          allow_cross_origin_credentials: false,
          provider_params: {},
          health: { status: health },
          created_at: 1,
          updated_at: 1,
        },
      ],
      created_at: 1,
      updated_at: 1,
    },
  ],
});

describe('exact Chat health lookup', () => {
  test('does not cross providers when model ids collide', () => {
    const providers = [provider(PROVIDER_A, 'healthy'), provider(PROVIDER_B, 'unhealthy')];

    expect(exactChatHealthDotColor(providers, PROVIDER_A, MODEL)).toBe('bg-green-500');
    expect(exactChatHealthDotColor(providers, PROVIDER_B, MODEL)).toBe('bg-red-500');
  });

  test('does not reuse another task capability health', () => {
    expect(exactChatHealthDotColor([provider(PROVIDER_A, 'healthy', 'embedding')], PROVIDER_A, MODEL)).toBeNull();
    expect(exactChatHealthDotColor([provider(PROVIDER_A, 'healthy')], PROVIDER_B, MODEL)).toBeNull();
  });
});
