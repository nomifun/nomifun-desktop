/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { getNomiToolGroupRuntimeState } from './useNomiMessage';

describe('getNomiToolGroupRuntimeState', () => {
  test('treats malformed tool_group data as inactive instead of calling array methods on it', () => {
    expect(getNomiToolGroupRuntimeState({ status: 'Executing' })).toEqual({
      tools: [],
      hasActive: false,
      hasAny: false,
      executingDescription: undefined,
    });
  });

  test('stringifies structured tool descriptions used in thought hints', () => {
    expect(
      getNomiToolGroupRuntimeState([
        {
          status: 'Executing',
          name: { label: 'Edit' },
          description: { file_path: 'src/App.tsx' },
        },
      ])
    ).toEqual({
      tools: [
        {
          status: 'Executing',
          name: '{\n  "label": "Edit"\n}',
          description: '{\n  "file_path": "src/App.tsx"\n}',
        },
      ],
      hasActive: true,
      hasAny: true,
      executingDescription: '{\n  "file_path": "src/App.tsx"\n}',
    });
  });
});

describe('useNomiMessage live event subscriptions', () => {
  test('subscribes to persisted user messages so IM-channel inbound turns appear without a DB reload', () => {
    const source = readFileSync(fileURLToPath(import.meta.resolve('./useNomiMessage.ts')), 'utf8');

    expect(source.includes('ipcBridge.conversation.userCreated.on')).toBe(true);
    expect(source.includes('transformUserCreatedEvent')).toBe(true);
  });

  test('treats output_discarded as an in-turn rollback boundary without clearing valid prefixes', () => {
    const source = readFileSync(fileURLToPath(import.meta.resolve('./useNomiMessage.ts')), 'utf8');
    const start = source.indexOf("case 'output_discarded':");
    const end = source.indexOf("case 'turn_completed':", start);
    const handler = source.slice(start, end);

    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    expect(handler.includes("dispatchTurnIfOpen({ type: 'activity' })")).toBe(true);
    expect(handler.includes("setThought({ subject: '', description: '' })")).toBe(true);
    expect(handler.includes('resetState')).toBe(false);
    expect(handler.includes('clearNomiMessageBuffer')).toBe(false);
  });
});
