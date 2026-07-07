/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { getProcessItemState, getToolMessagesProcessState } from './turnProcessState';

describe('turn process state', () => {
  test('treats tool confirmations as waiting for user input', () => {
    expect(
      getToolMessagesProcessState([
        {
          type: 'tool_group',
          content: [{ call_id: 'call-1', name: 'Edit', description: '', render_output_as_markdown: false, status: 'Confirming' }],
        } as any,
      ])
    ).toBe('waiting');
  });

  test('surfaces failed and canceled tool states', () => {
    expect(
      getToolMessagesProcessState([
        { type: 'tool_call', content: { call_id: 'call-1', name: 'Bash', args: {}, status: 'error' } } as any,
      ])
    ).toBe('failed');
    expect(
      getToolMessagesProcessState([
        {
          type: 'tool_group',
          content: [{ call_id: 'call-1', name: 'Edit', description: '', render_output_as_markdown: false, status: 'Canceled' }],
        } as any,
      ])
    ).toBe('canceled');
  });

  test('does not let non-fatal ACP shell command exits fail the whole process receipt', () => {
    expect(
      getToolMessagesProcessState([
        {
          type: 'acp_tool_call',
          content: {
            update: {
              sessionUpdate: 'tool_call_update',
              tool_call_id: 'call-bash',
              title: 'Bash',
              kind: 'execute',
              status: 'failed',
              rawInput: { command: 'grep -rn "missing" .' },
            },
          },
        } as any,
      ])
    ).toBe('completed');
  });

  test('does not let failed ACP read probes fail the whole process receipt', () => {
    expect(
      getToolMessagesProcessState([
        {
          type: 'acp_tool_call',
          content: {
            update: {
              sessionUpdate: 'tool_call_update',
              tool_call_id: 'call-read',
              title: 'config.yaml',
              kind: 'read',
              status: 'failed',
              rawInput: { path: 'config.yaml' },
            },
          },
        } as any,
      ])
    ).toBe('completed');
  });

  test('keeps permission and active thinking steps open', () => {
    expect(getProcessItemState({ type: 'permission' } as any)).toBe('waiting');
    expect(getProcessItemState({ type: 'thinking', content: { status: 'thinking' } } as any)).toBe('running');
  });

  test('marks error tips and agent errors as failed process evidence', () => {
    expect(getProcessItemState({ type: 'tips', content: { type: 'error' } } as any)).toBe('failed');
    expect(getProcessItemState({ type: 'agent_status', content: { status: 'error' } } as any)).toBe('failed');
  });

  test('keeps preparing agent status as a running process step', () => {
    expect(getProcessItemState({ type: 'agent_status', content: { status: 'preparing' } } as any)).toBe('running');
    expect(getProcessItemState({ type: 'agent_status', content: { status: 'prepared' } } as any)).toBe('completed');
  });
});
