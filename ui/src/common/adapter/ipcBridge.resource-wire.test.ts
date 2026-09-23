/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const bridgeSource = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');

describe('named resource wire IDs', () => {
  test('does not expose generic id parameters for core resource locators', () => {
    for (const expected of [
      '{ conversation_id: ConversationId }',
      '{ terminal_id: TerminalId }',
      '{ provider_id: ProviderId }',
      '{ knowledge_base_id: KnowledgeBaseId }',
    ]) {
      expect(bridgeSource.includes(expected)).toBe(true);
    }
    for (const legacy of [
      '/api/conversations/${p.id}',
      '/api/terminals/${p.id}',
      '/api/providers/${p.id}',
      '/api/knowledge/bases/${p.id}',
    ]) {
      expect(bridgeSource.includes(legacy)).toBe(false);
    }
  });
});
