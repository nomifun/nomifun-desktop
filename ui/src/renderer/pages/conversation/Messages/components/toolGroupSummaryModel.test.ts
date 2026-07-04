/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { NormalizedToolCall } from '@/common/chat/normalizeToolCall';
import { describe, expect, test } from 'bun:test';
import { buildToolReceiptSummaryParts, buildToolSummaryDescriptor } from './toolGroupSummaryModel';

const tool = (item: Partial<NormalizedToolCall> & Pick<NormalizedToolCall, 'key' | 'name'>): NormalizedToolCall => ({
  status: 'completed',
  ...item,
});

describe('buildToolReceiptSummaryParts', () => {
  test('summarizes mixed file reads and commands as separate receipt parts', () => {
    const parts = buildToolReceiptSummaryParts(
      [
        tool({ key: 'read-1', name: 'Read', description: 'MessageList.tsx' }),
        tool({ key: 'read-2', name: 'Read', description: 'messages.css' }),
        tool({ key: 'read-3', name: 'Read', description: 'turnDisclosureModel.ts' }),
        tool({ key: 'read-4', name: 'Read', description: 'toolGroupSummaryModel.ts' }),
        tool({ key: 'test', name: 'Bash', description: 'bun test ui/src/renderer/pages/conversation/Messages' }),
      ],
      'completed'
    );

    expect(parts).toEqual([
      { action: 'read_files', count: 4, state: 'completed' },
      {
        action: 'run_commands',
        count: 1,
        state: 'completed',
        target: 'bun test ui/src/renderer/pages/conversation/Messages',
      },
    ]);
  });

  test('keeps the command title preview for a single running command', () => {
    const parts = buildToolReceiptSummaryParts(
      [tool({ key: 'test', name: 'Bash', description: 'bun test turnDisclosureModel.test.ts', status: 'running' })],
      'running'
    );

    expect(parts).toEqual([
      { action: 'run_commands', count: 1, state: 'running', target: 'bun test turnDisclosureModel.test.ts' },
    ]);
  });

  test('recognizes code search and file listing as scan-friendly receipt titles', () => {
    const parts = buildToolReceiptSummaryParts(
      [
        tool({ key: 'rg', name: 'Grep', description: 'turnDisclosure' }),
        tool({ key: 'list', name: 'Glob', description: 'ui/src/**/*.tsx' }),
      ],
      'completed'
    );

    expect(parts).toEqual([
      { action: 'search_code', count: 1, state: 'completed' },
      { action: 'list_files', count: 1, state: 'completed' },
    ]);
  });

  test('keeps completed read status separate from a running command in the same receipt', () => {
    const parts = buildToolReceiptSummaryParts(
      [
        tool({ key: 'read', name: 'Read', description: 'MessageList.tsx', status: 'completed' }),
        tool({ key: 'test', name: 'Bash', description: 'bun test MessageList', status: 'running' }),
      ],
      'running'
    );

    expect(parts).toEqual([
      { action: 'read_files', count: 1, state: 'completed' },
      { action: 'run_commands', count: 1, state: 'running', target: 'bun test MessageList' },
    ]);
  });
});

describe('buildToolSummaryDescriptor', () => {
  test('focuses the active tool before older completed tools', () => {
    const descriptor = buildToolSummaryDescriptor(
      [
        tool({ key: 'read', name: 'Read', description: 'messages.css', status: 'completed' }),
        tool({ key: 'test', name: 'Bash', description: 'bun test ...', status: 'running' }),
      ],
      'running'
    );

    expect(descriptor?.target).toBe('Bash bun test ...');
    expect(descriptor?.count).toBe(2);
  });

  test('focuses failed tools when the group failed', () => {
    const descriptor = buildToolSummaryDescriptor(
      [
        tool({ key: 'read', name: 'Read', description: 'messages.css', status: 'completed' }),
        tool({ key: 'test', name: 'Bash', description: 'bun test ...', status: 'error' }),
      ],
      'failed'
    );

    expect(descriptor?.target).toBe('Bash bun test ...');
  });

  test('uses the latest completed tool for completed groups', () => {
    const descriptor = buildToolSummaryDescriptor(
      [
        tool({ key: 'read', name: 'Read', description: 'messages.css' }),
        tool({ key: 'edit', name: 'Edit', description: 'MessageList.tsx' }),
      ],
      'completed'
    );

    expect(descriptor?.target).toBe('Edit MessageList.tsx');
  });
});
