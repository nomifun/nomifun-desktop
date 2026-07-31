/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./useConversationListSync.ts', import.meta.url), 'utf8');

describe('conversation list reconnect recovery', () => {
  test('reloads the conversation list snapshot after websocket reconnect', () => {
    // WebSocket delivery has no replay: a conversation.listChanged frame lost
    // during a gap (delete/create while offline) is otherwise never recovered.
    expect(source.includes('ipcBridge.conversation.reconnected.on(() => refreshConversations())')).toBe(true);
  });
});
