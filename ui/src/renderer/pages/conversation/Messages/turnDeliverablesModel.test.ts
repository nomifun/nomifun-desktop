/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type {
  IMessageAcpToolCall,
  IMessageToolCall,
  IMessageToolGroup,
} from '@/common/chat/chatLib';
import type { PersistedToolArtifact } from '@/common/types/platform/acpTypes';
import { parseMessageId } from '@/common/types/ids';
import { parseDiff } from '@/renderer/utils/file/diffUtils';
import {
  collectTurnDeliverables,
  type TurnDeliverableCandidate,
  type TurnGateInfo,
} from './turnDeliverablesModel';

const TURN_1 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000001');
const TURN_2 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000002');
const MSG_1 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000021');
const MSG_2 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000022');
const MSG_3 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000023');
const ARTIFACT_ID = '0190f5fe-7c00-7a00-8000-000000000031';

const WORKSPACE = 'C:/data/conversations/ws-1';

const artifact = (overrides: Partial<PersistedToolArtifact> = {}): PersistedToolArtifact =>
  ({
    id: ARTIFACT_ID,
    kind: 'file',
    mime_type: 'text/html',
    path: `${WORKSPACE}/outputs/report.html`,
    relative_path: 'outputs/report.html',
    size_bytes: 2048,
    sha256: 'a'.repeat(64),
    ...overrides,
  }) as PersistedToolArtifact;

const toolCall = (
  content: Partial<IMessageToolCall['content']>,
  overrides: Partial<IMessageToolCall> = {}
): IMessageToolCall =>
  ({
    id: 'local-1',
    conversation_id: 'conv-1',
    type: 'tool_call',
    message_id: MSG_1,
    turn_id: TURN_1,
    content: { call_id: 'call-1', name: 'Write', status: 'completed', ...content },
    ...overrides,
  }) as unknown as IMessageToolCall;

const acpToolCall = (
  update: Partial<IMessageAcpToolCall['content']['update']>,
  overrides: Partial<IMessageAcpToolCall> = {}
): IMessageAcpToolCall =>
  ({
    id: 'local-2',
    conversation_id: 'conv-1',
    type: 'acp_tool_call',
    message_id: MSG_2,
    turn_id: TURN_1,
    content: {
      session_id: 'sess-1',
      update: {
        sessionUpdate: 'tool_call',
        tool_call_id: 'acp-call-1',
        status: 'completed',
        ...update,
      },
    },
    ...overrides,
  }) as unknown as IMessageAcpToolCall;

const toolGroup = (
  items: Array<Record<string, unknown>>,
  overrides: Partial<IMessageToolGroup> = {}
): IMessageToolGroup =>
  ({
    id: 'local-3',
    conversation_id: 'conv-1',
    type: 'tool_group',
    message_id: MSG_3,
    turn_id: TURN_1,
    content: items,
    ...overrides,
  }) as unknown as IMessageToolGroup;

const WRITE_FILE_DIFF = [
  'diff --git a/outputs/snake.html b/outputs/snake.html',
  'index 000..111 100644',
  '--- a/outputs/snake.html',
  '+++ b/outputs/snake.html',
  '@@ -0,0 +1,3 @@',
  '+<html>',
  '+<body>snake</body>',
  '+</html>',
].join('\n');

const candidate = (
  overrides: Partial<TurnDeliverableCandidate> = {}
): TurnDeliverableCandidate => ({
  turnId: TURN_1,
  role: 'process',
  processState: 'completed',
  ...overrides,
});

const openGate = (): Map<string, TurnGateInfo> =>
  new Map([[TURN_1 as string, { running: false, state: 'completed' as const }]]);

const collect = (
  candidates: TurnDeliverableCandidate[],
  gates: Map<string, TurnGateInfo> = openGate()
) =>
  collectTurnDeliverables(candidates, {
    workspaceRoots: [WORKSPACE],
    turnGates: gates,
  });

describe('collectTurnDeliverables', () => {
  test('collects committed receipt artifacts from nomi tool_call', () => {
    const result = collect([
      candidate({ toolMessages: [toolCall({ artifacts: [artifact()] })] }),
    ]);

    const items = result.get(TURN_1);
    expect(items).toBeDefined();
    expect(items).toHaveLength(1);
    expect(items![0]).toMatchObject({
      relativePath: 'outputs/report.html',
      fileName: 'report.html',
      absolutePath: `${WORKSPACE}/outputs/report.html`,
      sizeBytes: 2048,
      tier: 'receipt',
    });
    expect(items![0].sources[0]).toMatchObject({ carrier: 'tool_call_artifact', callId: 'call-1' });
    expect(items![0].sources[0].sourceMessageIds).toEqual([MSG_1]);
  });

  test('skips artifacts of non-completed tool calls', () => {
    const result = collect([
      candidate({
        processState: 'failed',
        toolMessages: [toolCall({ status: 'error', artifacts: [artifact()] })],
      }),
      candidate({ role: 'assistant', processState: 'completed' }),
    ]);
    expect(result.size).toBe(0);
  });

  test('extracts reported edit targets from nomi Write/Edit/ApplyPatch args', () => {
    const result = collect([
      candidate({
        toolMessages: [
          toolCall({ name: 'Write', args: { file_path: `${WORKSPACE}/outputs/a.md`, content: 'x' } }),
          toolCall(
            { call_id: 'call-2', name: 'ApplyPatch', args: { files: [
              { file_path: `${WORKSPACE}/outputs/b.md` },
              { file_path: `${WORKSPACE}/outputs/gone.md`, delete: true },
            ] } },
            { message_id: MSG_2 }
          ),
        ],
      }),
    ]);

    const items = result.get(TURN_1);
    expect(items?.map((item) => item.relativePath)).toEqual(['outputs/a.md', 'outputs/b.md']);
    expect(items?.every((item) => item.tier === 'reported')).toBe(true);
  });

  test('does not treat read-like or shell tools as deliverables', () => {
    const result = collect([
      candidate({
        toolMessages: [
          toolCall({ name: 'Read', args: { file_path: `${WORKSPACE}/outputs/a.md` } }),
          toolCall({ call_id: 'call-2', name: 'Bash', args: { command: 'ls' } }),
          toolCall({ call_id: 'call-3', name: 'write_stdin', args: { path: 'x' } }),
        ],
      }),
    ]);
    expect(result.size).toBe(0);
  });

  test('collects ACP diff content with computed line counts', () => {
    const result = collect([
      candidate({
        toolMessages: [
          acpToolCall({
            kind: 'edit',
            content: [
              { type: 'diff', path: `${WORKSPACE}/outputs/snake.html`, old_text: 'a\nb\n', new_text: 'a\nc\nd\n' },
            ],
          }),
        ],
      }),
    ]);

    const items = result.get(TURN_1);
    expect(items).toHaveLength(1);
    expect(items![0].relativePath).toBe('outputs/snake.html');
    expect(items![0].tier).toBe('reported');
    expect(items![0].insertions).toBe(2);
    expect(items![0].deletions).toBe(1);
    expect(items![0].diff?.includes('snake.html')).toBe(true);
  });

  test('collects ACP artifact content items as receipts', () => {
    const result = collect([
      candidate({
        toolMessages: [
          acpToolCall({ content: [{ type: 'artifact', artifact: artifact() }] }),
        ],
      }),
    ]);
    const items = result.get(TURN_1);
    expect(items).toHaveLength(1);
    expect(items![0].tier).toBe('receipt');
    expect(items![0].sha256).toBe('a'.repeat(64));
  });

  test('falls back to ACP rawInput/locations for edit kind without diff content', () => {
    const result = collect([
      candidate({
        toolMessages: [
          acpToolCall({
            kind: 'edit',
            rawInput: { file_path: `${WORKSPACE}/outputs/x.ts` },
            locations: [{ path: `${WORKSPACE}/outputs/y.ts` }],
          }),
        ],
      }),
    ]);
    const items = result.get(TURN_1);
    expect(items?.map((item) => item.relativePath).sort()).toEqual(['outputs/x.ts', 'outputs/y.ts']);
  });

  test('ignores ACP locations for non-edit kinds', () => {
    const result = collect([
      candidate({
        toolMessages: [
          acpToolCall({ kind: 'read', locations: [{ path: `${WORKSPACE}/outputs/y.ts` }] }),
        ],
      }),
    ]);
    expect(result.size).toBe(0);
  });

  test('collects successful WriteFile tool_group results with diff stats', () => {
    const result = collect([
      candidate({
        toolMessages: [
          toolGroup([
            {
              call_id: 'g-1',
              name: 'WriteFile',
              status: 'Success',
              description: '',
              render_output_as_markdown: false,
              result_display: { file_diff: WRITE_FILE_DIFF, file_name: 'outputs/snake.html' },
            },
            {
              call_id: 'g-2',
              name: 'WriteFile',
              status: 'Error',
              description: '',
              render_output_as_markdown: false,
              result_display: { file_diff: WRITE_FILE_DIFF, file_name: 'outputs/failed.html' },
            },
          ]),
        ],
      }),
    ]);

    const items = result.get(TURN_1);
    expect(items).toHaveLength(1);
    expect(items![0]).toMatchObject({ relativePath: 'outputs/snake.html', insertions: 3, deletions: 0 });
  });

  test('collects pre-parsed file_summary diffs', () => {
    const info = parseDiff(WRITE_FILE_DIFF, 'outputs/snake.html');
    const result = collect([
      candidate({ fileDiffs: [info], fileDiffSourceMessageIds: [MSG_1] }),
    ]);
    const items = result.get(TURN_1);
    expect(items).toHaveLength(1);
    expect(items![0].sources[0].carrier).toBe('write_file_diff');
    expect(items![0].sources[0].sourceMessageIds).toEqual([MSG_1]);
  });

  test('dedupes by workspace-relative path keeping the last write and merging receipt data', () => {
    const result = collect([
      candidate({
        toolMessages: [
          acpToolCall({
            kind: 'edit',
            content: [
              { type: 'diff', path: `${WORKSPACE}/outputs/report.html`, old_text: '', new_text: 'v1\n' },
            ],
          }),
          acpToolCall(
            {
              tool_call_id: 'acp-call-2',
              kind: 'edit',
              content: [
                { type: 'diff', path: 'outputs\\report.html', old_text: 'v1\n', new_text: 'v2\nv3\n' },
              ],
            },
            { message_id: MSG_3 }
          ),
          toolCall({ artifacts: [artifact()] }),
        ],
      }),
    ]);

    const items = result.get(TURN_1);
    expect(items).toHaveLength(1);
    expect(items![0].tier).toBe('receipt');
    expect(items![0].sizeBytes).toBe(2048);
    expect(items![0].insertions).toBe(2);
    expect(items![0].deletions).toBe(1);
    expect(items![0].sources).toHaveLength(3);
  });

  test('returns no card for running, canceled or missing-gate turns', () => {
    const deliverable = candidate({ toolMessages: [toolCall({ artifacts: [artifact()] })] });

    expect(
      collect([deliverable], new Map([[TURN_1 as string, { running: true, state: 'running' as const }]])).size
    ).toBe(0);
    expect(
      collect([deliverable], new Map([[TURN_1 as string, { running: false, state: 'canceled' as const }]])).size
    ).toBe(0);
    expect(collect([deliverable], new Map()).size).toBe(0);
  });

  test('returns no card when the turn ends in a terminal failure', () => {
    const result = collect([
      candidate({ toolMessages: [toolCall({ artifacts: [artifact()] })] }),
      candidate({ role: 'assistant', processState: 'failed' }),
    ]);
    expect(result.size).toBe(0);
  });

  test('keeps the card when a mid-turn failure recovers before completion', () => {
    const result = collect([
      candidate({ processState: 'failed', toolMessages: [toolCall({ status: 'error' })] }),
      candidate({ toolMessages: [toolCall({ call_id: 'call-9', artifacts: [artifact()] })] }),
      candidate({ role: 'assistant', processState: 'completed' }),
    ]);
    expect(result.get(TURN_1)).toHaveLength(1);
  });

  test('groups deliverables per turn independently', () => {
    const gates = new Map<string, TurnGateInfo>([
      [TURN_1 as string, { running: false, state: 'completed' }],
      [TURN_2 as string, { running: false, state: 'completed' }],
    ]);
    const result = collect(
      [
        candidate({ toolMessages: [toolCall({ artifacts: [artifact()] })] }),
        candidate({
          turnId: TURN_2,
          toolMessages: [
            toolCall(
              { call_id: 'call-2', artifacts: [artifact({ relative_path: 'outputs/two.png', path: `${WORKSPACE}/outputs/two.png`, kind: 'image' })] },
              { message_id: MSG_2, turn_id: TURN_2 }
            ),
          ],
        }),
      ],
      gates
    );

    expect(result.get(TURN_1)?.[0].relativePath).toBe('outputs/report.html');
    expect(result.get(TURN_2)?.[0].relativePath).toBe('outputs/two.png');
  });

  test('items without a turn id are ignored', () => {
    const result = collect([
      candidate({ turnId: undefined, toolMessages: [toolCall({ artifacts: [artifact()] })] }),
    ]);
    expect(result.size).toBe(0);
  });

  test('normalizes windows separators and keeps outside-workspace paths absolute', () => {
    const result = collect([
      candidate({
        toolMessages: [
          toolCall({ name: 'Write', args: { file_path: 'C:\\data\\conversations\\ws-1\\outputs\\win.md' } }),
          toolCall(
            { call_id: 'call-2', name: 'Write', args: { file_path: 'D:\\elsewhere\\out.md' } },
            { message_id: MSG_2 }
          ),
        ],
      }),
    ]);

    const items = result.get(TURN_1);
    expect(items?.map((item) => item.relativePath)).toEqual(['outputs/win.md', 'D:/elsewhere/out.md']);
    expect(items?.[1].absolutePath).toBe('D:/elsewhere/out.md');
  });
});
