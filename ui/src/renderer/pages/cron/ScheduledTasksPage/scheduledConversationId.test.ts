/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseScheduledConversationId } from './scheduledConversationId';

describe('parseScheduledConversationId', () => {
  test('parses a locked conversation id from route search params', () => {
    const conversationId = parseScheduledConversationId(
      new URLSearchParams('create=conversation&conversation_id=42'),
    );

    expect(conversationId).toBe(42);
  });

  test('ignores invalid or incomplete conversation ids', () => {
    expect(parseScheduledConversationId(new URLSearchParams('create=conversation'))).toBeNull();
    expect(
      parseScheduledConversationId(new URLSearchParams('create=conversation&conversation_id=abc')),
    ).toBeNull();
    expect(parseScheduledConversationId(new URLSearchParams('create=unknown&conversation_id=42'))).toBeNull();
  });
});
