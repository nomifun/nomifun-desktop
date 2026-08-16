/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type {
  IMessageToolCall,
  IMessageToolGroup,
} from '@/common/chat/chatLib';
import { normalizeToolCallContent } from '@/common/chat/chatLib';
import type { PersistedToolArtifact } from '@/common/types/platform/toolCallTypes';
import { parseMessageId, parsePersistedArtifactId } from '@/common/types/ids';
import { parseDiff } from '@/renderer/utils/file/diffUtils';
import {
  collectTurnDeliverables,
  isVerifiedImageDeliverable,
  type TurnDeliverableCandidate,
  type TurnGateInfo,
} from './turnDeliverablesModel';

const TURN_1 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000001');
const TURN_2 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000002');
const MSG_1 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000021');
const MSG_2 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000022');
const MSG_3 = parseMessageId('0190f5fe-7c00-7a00-8000-000000000023');
const ARTIFACT_ID = parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000031');
const IMAGE_ARTIFACT_ID = parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000032');
const FILE_ARTIFACT_ID = parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000033');
const WRONG_MIME_ARTIFACT_ID = parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000034');
const SECOND_IMAGE_ARTIFACT_ID = parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000035');

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

const REPORT_DIFF_V1 = [
  'diff --git a/outputs/report.html b/outputs/report.html',
  '--- a/outputs/report.html',
  '+++ b/outputs/report.html',
  '@@ -0,0 +1,1 @@',
  '+v1',
].join('\n');

const REPORT_DIFF_V2 = [
  'diff --git a/outputs/report.html b/outputs/report.html',
  '--- a/outputs/report.html',
  '+++ b/outputs/report.html',
  '@@ -1,1 +1,2 @@',
  '-v1',
  '+v2',
  '+v3',
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
      artifactId: ARTIFACT_ID,
      artifactKind: 'file',
      mimeType: 'text/html',
      tier: 'receipt',
    });
    expect(items![0].sources[0]).toMatchObject({ carrier: 'tool_call_artifact', callId: 'call-1' });
    expect(items![0].sources[0].sourceMessageIds).toEqual([MSG_1]);
  });

  test('preserves canonical UNC and extended-length receipt paths for native actions', () => {
    const uncPath = String.raw`\\server\share\nomifun-artifacts\generated.png`;
    const extendedPath = String.raw`\\?\C:\nomifun-artifacts\generated-2.png`;
    const result = collect([
      candidate({
        toolMessages: [
          toolCall({
            artifacts: [
              artifact({
                id: IMAGE_ARTIFACT_ID,
                kind: 'image',
                mime_type: 'image/png',
                path: uncPath,
                relative_path: 'generated.png',
              }),
              artifact({
                id: FILE_ARTIFACT_ID,
                kind: 'image',
                mime_type: 'image/png',
                path: extendedPath,
                relative_path: 'generated-2.png',
              }),
            ],
          }),
        ],
      }),
    ]);

    expect(result.get(TURN_1)?.map((item) => item.absolutePath)).toEqual([
      uncPath,
      extendedPath,
    ]);
  });

  test('never lets a later reported draft replace a committed receipt locator', () => {
    const canonicalPath = `${WORKSPACE}/outputs/generated.png`;
    const result = collect([
      candidate({
        toolMessages: [
          toolCall({
            artifacts: [
              artifact({
                id: IMAGE_ARTIFACT_ID,
                kind: 'image',
                mime_type: 'image/png',
                path: canonicalPath,
                relative_path: 'outputs/generated.png',
              }),
            ],
          }),
          toolCall(
            {
              call_id: 'reported-after-receipt',
              name: 'Write',
              args: { file_path: 'c:\\DATA\\CONVERSATIONS\\WS-1\\OUTPUTS\\GENERATED.PNG' },
            },
            { message_id: MSG_2 }
          ),
        ],
      }),
    ]);

    const verified = result.get(TURN_1)?.find(isVerifiedImageDeliverable);
    expect(verified?.absolutePath).toBe(canonicalPath);
    expect(verified?.artifactId).toBe(IMAGE_ARTIFACT_ID);
    expect(verified?.sha256).toBe('a'.repeat(64));
  });

  test('keeps same-relative-path receipts from different workspace roots separate', () => {
    const secondRoot = 'D:/other-workspace';
    const gates = openGate();
    const result = collectTurnDeliverables(
      [
        candidate({
          toolMessages: [
            toolCall({
              artifacts: [
                artifact({
                  id: IMAGE_ARTIFACT_ID,
                  kind: 'image',
                  mime_type: 'image/png',
                  path: `${WORKSPACE}/outputs/generated.png`,
                  relative_path: 'outputs/generated.png',
                }),
                artifact({
                  id: SECOND_IMAGE_ARTIFACT_ID,
                  kind: 'image',
                  mime_type: 'image/png',
                  path: `${secondRoot}/outputs/generated.png`,
                  relative_path: 'outputs/generated.png',
                }),
              ],
            }),
          ],
        }),
      ],
      { workspaceRoots: [WORKSPACE, secondRoot], turnGates: gates }
    );

    expect(result.get(TURN_1)?.map((item) => item.absolutePath)).toEqual([
      `${WORKSPACE}/outputs/generated.png`,
      `${secondRoot}/outputs/generated.png`,
    ]);
  });

  test('keeps the first canonical proof when one artifact id is reported with conflicting paths', () => {
    const canonicalPath = `${WORKSPACE}/outputs/canonical.png`;
    const result = collect([
      candidate({
        toolMessages: [
          toolCall({
            artifacts: [
              artifact({
                id: IMAGE_ARTIFACT_ID,
                kind: 'image',
                mime_type: 'image/png',
                path: canonicalPath,
                relative_path: 'outputs/canonical.png',
              }),
              artifact({
                id: IMAGE_ARTIFACT_ID,
                kind: 'image',
                mime_type: 'image/png',
                path: 'D:/unexpected/conflict.png',
                relative_path: 'conflict.png',
              }),
            ],
          }),
        ],
      }),
    ]);

    expect(result.get(TURN_1)).toHaveLength(1);
    expect(result.get(TURN_1)?.[0].absolutePath).toBe(canonicalPath);
  });

  test('does not case-fold POSIX receipt paths', () => {
    const result = collectTurnDeliverables(
      [
        candidate({
          toolMessages: [
            toolCall({
              artifacts: [
                artifact({
                  id: IMAGE_ARTIFACT_ID,
                  kind: 'image',
                  mime_type: 'image/png',
                  path: '/work/outputs/A.png',
                  relative_path: 'outputs/A.png',
                }),
                artifact({
                  id: SECOND_IMAGE_ARTIFACT_ID,
                  kind: 'image',
                  mime_type: 'image/png',
                  path: '/work/outputs/a.png',
                  relative_path: 'outputs/a.png',
                }),
              ],
            }),
          ],
        }),
      ],
      { workspaceRoots: ['/work'], turnGates: openGate() }
    );

    expect(result.get(TURN_1)).toHaveLength(2);
  });

  test('admits only receipt-declared image artifacts to the first-class image result', () => {
    const result = collect([
      candidate({
        toolMessages: [
          toolCall({
            artifacts: [
              artifact({
                id: IMAGE_ARTIFACT_ID,
                kind: 'image',
                mime_type: 'image/png',
                path: `${WORKSPACE}/outputs/generated.png`,
                relative_path: 'outputs/generated.png',
              }),
              artifact({
                id: FILE_ARTIFACT_ID,
                kind: 'file',
                mime_type: 'image/png',
                path: `${WORKSPACE}/outputs/file-labelled.png`,
                relative_path: 'outputs/file-labelled.png',
              }),
              artifact({
                id: WRONG_MIME_ARTIFACT_ID,
                kind: 'image',
                mime_type: 'application/octet-stream',
                path: `${WORKSPACE}/outputs/wrong-mime.png`,
                relative_path: 'outputs/wrong-mime.png',
              }),
            ],
          }),
          toolCall({
            call_id: 'call-2',
            name: 'Write',
            args: { file_path: `${WORKSPACE}/outputs/reported-only.png` },
          }),
        ],
      }),
    ]);

    const items = result.get(TURN_1) ?? [];
    expect(items.filter(isVerifiedImageDeliverable).map((item) => item.relativePath)).toEqual([
      'outputs/generated.png',
    ]);
    expect(items.find((item) => item.relativePath === 'outputs/generated.png')).toMatchObject({
      artifactId: IMAGE_ARTIFACT_ID,
      artifactKind: 'image',
      mimeType: 'image/png',
      tier: 'receipt',
    });
  });

  test('does not surface an uncommitted persisted image claim', () => {
    const claimedImage = artifact({
      id: IMAGE_ARTIFACT_ID,
      kind: 'image',
      mime_type: 'image/png',
      path: `${WORKSPACE}/outputs/uncommitted.png`,
      relative_path: 'outputs/uncommitted.png',
    });
    const normalized = normalizeToolCallContent(
      {
        call_id: 'image-call',
        name: 'image_gen',
        status: 'completed',
        artifacts: [claimedImage],
        artifact_delivery_committed: false,
      },
      'finish'
    );

    expect(normalized.status).toBe('error');
    expect(normalized.artifacts).toEqual([]);
    expect(collect([candidate({ toolMessages: [toolCall(normalized)] })]).size).toBe(0);
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
          toolGroup([
            {
              call_id: 'g-1',
              name: 'WriteFile',
              status: 'Success',
              description: '',
              render_output_as_markdown: false,
              result_display: { file_diff: REPORT_DIFF_V1, file_name: 'outputs/report.html' },
            },
            {
              call_id: 'g-2',
              name: 'WriteFile',
              status: 'Success',
              description: '',
              render_output_as_markdown: false,
              result_display: { file_diff: REPORT_DIFF_V2, file_name: 'outputs/report.html' },
            },
          ]),
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

  test('normalizes windows separators and hides outside-workspace absolute roots from display', () => {
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
    expect(items?.map((item) => item.relativePath)).toEqual(['outputs/win.md', 'out.md']);
    expect(items?.[1].absolutePath).toBe('D:/elsewhere/out.md');
  });

  test('does not treat a differently-cased POSIX root as the active workspace', () => {
    const result = collectTurnDeliverables(
      [
        candidate({
          toolMessages: [
            toolCall({ name: 'Write', args: { file_path: '/work/outputs/case.md' } }),
          ],
        }),
      ],
      { workspaceRoots: ['/Work'], turnGates: openGate() }
    );

    expect(result.get(TURN_1)?.[0]).toMatchObject({
      relativePath: 'case.md',
      absolutePath: '/work/outputs/case.md',
    });
  });

  test('keeps same-named files outside the workspace as separate deliverables', () => {
    const result = collect([
      candidate({
        toolMessages: [
          toolCall({ name: 'Write', args: { file_path: 'D:/reports/out.md' } }),
          toolCall(
            { call_id: 'call-2', name: 'Write', args: { file_path: 'E:/exports/out.md' } },
            { message_id: MSG_2 }
          ),
        ],
      }),
    ]);

    const items = result.get(TURN_1);
    expect(items).toHaveLength(2);
    expect(items?.map((item) => item.relativePath)).toEqual(['out.md', 'out.md']);
    expect(items?.map((item) => item.absolutePath)).toEqual(['D:/reports/out.md', 'E:/exports/out.md']);
  });
});
