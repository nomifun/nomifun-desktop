/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { NormalizedToolCall } from '@/common/chat/normalizeToolCall';
import { describe, expect, test } from 'bun:test';
import { buildToolSummaryDescriptor } from './toolGroupSummaryModel';

const tool = (item: Partial<NormalizedToolCall> & Pick<NormalizedToolCall, 'key' | 'name'>): NormalizedToolCall => ({
  status: 'completed',
  ...item,
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
