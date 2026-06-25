/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseScheduledCreateTarget } from './scheduledCreateTarget';

describe('parseScheduledCreateTarget', () => {
  test('parses a locked conversation create target from route search params', () => {
    const target = parseScheduledCreateTarget(new URLSearchParams('create=conversation&conversation_id=42'));

    expect(target).toEqual({ kind: 'conversation', conversationId: 42 });
  });

  test('ignores invalid or incomplete create targets', () => {
    expect(parseScheduledCreateTarget(new URLSearchParams('create=conversation'))).toBeNull();
    expect(parseScheduledCreateTarget(new URLSearchParams('create=conversation&conversation_id=abc'))).toBeNull();
    expect(parseScheduledCreateTarget(new URLSearchParams('create=terminal&conversation_id=42'))).toBeNull();
  });
});
