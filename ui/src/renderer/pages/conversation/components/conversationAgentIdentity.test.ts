/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { officialConversationTemplateKey } from './conversationAgentIdentity';

describe('officialConversationTemplateKey', () => {
  test('uses only verified host projection, not a matching display name', () => {
    expect(officialConversationTemplateKey({ official_template_key: 'chat.minimal' })).toBe('chat.minimal');
    expect(officialConversationTemplateKey({ agent_name: 'chat.minimal' })).toBeNull();
    expect(officialConversationTemplateKey({ official_template_key: 'customer-service.default' })).toBeNull();
    expect(officialConversationTemplateKey({ official_template_key: 'unknown.template' })).toBeNull();
  });
});
