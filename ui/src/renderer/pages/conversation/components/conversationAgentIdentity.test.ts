/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { officialAgentTemplateKey, officialConversationTemplateKey } from './conversationAgentIdentity';

describe('officialConversationTemplateKey', () => {
  test('uses only verified host projection, not a matching display name', () => {
    expect(officialConversationTemplateKey({ official_template_key: 'chat.minimal' })).toBe('chat.minimal');
    expect(officialConversationTemplateKey({ agent_name: 'chat.minimal' })).toBeNull();
    expect(officialConversationTemplateKey({ official_template_key: 'customer-service.default' })).toBeNull();
    expect(officialConversationTemplateKey({ official_template_key: 'unknown.template' })).toBeNull();
  });
});

test('presentation recognizes product Agents without exposing them in ordinary chat selection', () => {
  for (const key of ['companion.default', 'companion.robot', 'customer-service.default'] as const) {
    expect(officialAgentTemplateKey({ official_template_key: key })).toBe(key);
    expect(officialConversationTemplateKey({ official_template_key: key })).toBeNull();
    expect(officialAgentTemplateKey({ agent_name: key })).toBeNull();
  }
  for (const extra of [null, [], 'companion.default', { official_template_key: 'unknown.template' }]) {
    expect(officialAgentTemplateKey(extra)).toBeNull();
  }
});
