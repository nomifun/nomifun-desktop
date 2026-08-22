/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { CreativeStudioContractError } from '../../../domain';
import type { CreativeCanvasAgentOp } from './artifacts';
import {
  createCreativeCanvasAgentOpsPort,
  type CreativeCanvasAgentOpResult,
  type CreativeCanvasAgentOpsHttpRequest,
} from './opsPort';

const PROJECT_ID = '0190f5fe-7c00-7a00-8000-000000000901';
const OTHER_PROJECT_ID = '0190f5fe-7c00-7a00-8000-000000000902';
const ASSISTANT_MESSAGE_ID = '0190f5fe-7c00-7a00-8000-000000000908';
const NODE_A = '0190f5fe-7c00-7a00-8000-000000000903';
const NODE_B = '0190f5fe-7c00-7a00-8000-000000000904';
const ADDED_NODE = '0190f5fe-7c00-7a00-8000-000000000905';
const CONNECTION = '0190f5fe-7c00-7a00-9000-000000000906';
const ADDED_CONNECTION = '0190f5fe-7c00-7a00-9000-000000000907';

const ops: CreativeCanvasAgentOp[] = [
  {
    type: 'add_node',
    node_type: 'text',
    x: 10,
    y: 20,
    data: { text: 'Title', format: 'plain', fontSize: 18, textAlign: 'left' },
  },
  { type: 'update_node_data', node_id: NODE_A, patch: { textAlign: 'center' } },
  { type: 'move_node', node_id: NODE_A, x: -100, y: 80 },
  { type: 'resize_node', node_id: NODE_A, width: 320, height: 180 },
  {
    type: 'connect',
    source_node_id: NODE_A,
    target_node_id: NODE_B,
    source_handle: null,
  },
  { type: 'disconnect', connection_id: CONNECTION },
];

const results: CreativeCanvasAgentOpResult[] = [
  { type: 'node_added', node_id: ADDED_NODE },
  { type: 'node_updated', node_id: NODE_A },
  { type: 'node_moved', node_id: NODE_A },
  { type: 'node_resized', node_id: NODE_A },
  { type: 'nodes_connected', connection_id: ADDED_CONNECTION },
  { type: 'nodes_disconnected', connection_id: CONNECTION },
];

const summary = (overrides: Record<string, unknown> = {}) => ({
  projectId: PROJECT_ID,
  title: 'Agent canvas',
  revision: '8',
  nodeCount: 3,
  connectionCount: 1,
  createdAt: 1_770_000_000_000,
  updatedAt: 1_770_000_001_000,
  ...overrides,
});

const appliedResponse = (overrides: Record<string, unknown> = {}) => ({
  project: summary(),
  ops: results,
  replayed: false,
  appliedRevision: '8',
  ...overrides,
});

const captureAsyncError = async (run: () => Promise<unknown>): Promise<unknown> => {
  try {
    await run();
    return null;
  } catch (error) {
    return error;
  }
};

describe('Creative Canvas Agent operations HTTP port', () => {
  test('posts the exact canonical request and parses the direct unwrapped response', async () => {
    const calls: Array<{ method: string; path: string; body?: unknown }> = [];
    const request: CreativeCanvasAgentOpsHttpRequest = async (method, path, body) => {
      calls.push({ method, path, body });
      return appliedResponse();
    };

    const applied = await createCreativeCanvasAgentOpsPort(request).apply({
      projectId: PROJECT_ID,
      assistantMessageId: ASSISTANT_MESSAGE_ID,
      expectedRevision: '7',
      ops,
    });

    expect(calls).toEqual([
      {
        method: 'POST',
        path: `/api/creative-studio/projects/${PROJECT_ID}/agent-ops`,
        body: { assistantMessageId: ASSISTANT_MESSAGE_ID, expectedRevision: '7', ops },
      },
    ]);
    expect(applied).toEqual(appliedResponse());
  });

  test('preserves stale, server and response-loss errors without an automatic retry', async () => {
    for (const failure of [
      Object.assign(new Error('stale revision'), { status: 409, code: 'REVISION_CONFLICT' }),
      Object.assign(new Error('backend unavailable'), { status: 503 }),
      new TypeError('connection lost after request delivery'),
    ]) {
      let calls = 0;
      const request: CreativeCanvasAgentOpsHttpRequest = async () => {
        calls += 1;
        throw failure;
      };
      const caught = await captureAsyncError(() =>
        createCreativeCanvasAgentOpsPort(request).apply({
          projectId: PROJECT_ID,
          assistantMessageId: ASSISTANT_MESSAGE_ID,
          expectedRevision: '7',
          ops: [ops[2]!],
        })
      );
      expect(caught).toBe(failure);
      expect(calls).toBe(1);
    }
  });

  test('accepts a durable replay without requiring a second revision increment', async () => {
    const replayed = await createCreativeCanvasAgentOpsPort(async () =>
      appliedResponse({
        project: summary({ revision: '12' }),
        ops: [results[2]],
        replayed: true,
        appliedRevision: '8',
      })
    ).apply({
      projectId: PROJECT_ID,
      assistantMessageId: ASSISTANT_MESSAGE_ID,
      expectedRevision: '12',
      ops: [ops[2]!],
    });

    expect(replayed).toEqual(
      appliedResponse({
        project: summary({ revision: '12' }),
        ops: [results[2]],
        replayed: true,
        appliedRevision: '8',
      })
    );

    const regressed = createCreativeCanvasAgentOpsPort(async () =>
      appliedResponse({
        project: summary({ revision: '7' }),
        ops: [results[2]],
        replayed: true,
        appliedRevision: '8',
      })
    );
    const error = await captureAsyncError(() =>
      regressed.apply({
        projectId: PROJECT_ID,
        assistantMessageId: ASSISTANT_MESSAGE_ID,
        expectedRevision: '7',
        ops: [ops[2]!],
      })
    );
    expect(error instanceof CreativeStudioContractError).toBe(true);
    expect((error as CreativeStudioContractError).path).toBe('$.project.revision');
  });

  test('rejects a foreign project and a revision that did not advance exactly once', async () => {
    const foreign = createCreativeCanvasAgentOpsPort(async () =>
      appliedResponse({
        project: summary({ projectId: OTHER_PROJECT_ID }),
        ops: [results[2]],
      })
    );
    const foreignError = await captureAsyncError(() =>
      foreign.apply({
        projectId: PROJECT_ID,
        assistantMessageId: ASSISTANT_MESSAGE_ID,
        expectedRevision: '7',
        ops: [ops[2]!],
      })
    );
    expect(foreignError instanceof CreativeStudioContractError).toBe(true);
    expect((foreignError as CreativeStudioContractError).code).toBe('PROJECT_MISMATCH');

    for (const revision of ['7', '9', '9223372036854775807']) {
      const invalidRevision = createCreativeCanvasAgentOpsPort(async () =>
        appliedResponse({
          project: summary({ revision }),
          ops: [results[2]],
          appliedRevision: revision,
        })
      );
      const error = await captureAsyncError(() =>
        invalidRevision.apply({
          projectId: PROJECT_ID,
          assistantMessageId: ASSISTANT_MESSAGE_ID,
          expectedRevision: '7',
          ops: [ops[2]!],
        })
      );
      expect(error instanceof CreativeStudioContractError).toBe(true);
      expect((error as CreativeStudioContractError).path).toBe('$.appliedRevision');
    }

    const largeRevision = '9007199254740993';
    const preciseRevision = (BigInt(largeRevision) + 1n).toString();
    const precise = createCreativeCanvasAgentOpsPort(async () =>
      appliedResponse({
        project: summary({ revision: preciseRevision }),
        ops: [results[2]],
        appliedRevision: preciseRevision,
      })
    );
    const largeApplied = await precise.apply({
      projectId: PROJECT_ID,
      assistantMessageId: ASSISTANT_MESSAGE_ID,
      expectedRevision: largeRevision,
      ops: [ops[2]!],
    });
    expect(largeApplied).toEqual({
      project: summary({ revision: preciseRevision }),
      ops: [results[2]],
      replayed: false,
      appliedRevision: preciseRevision,
    });
  });

  test('rejects missing, extra, unknown and operation-mismatched results', async () => {
    const invalidResponses: unknown[] = [
      appliedResponse({ ops: [] }),
      appliedResponse({ ops: [results[2], results[2]] }),
      appliedResponse({ ops: [{ type: 'node_deleted', node_id: NODE_A }] }),
      appliedResponse({ ops: [{ type: 'node_moved', node_id: NODE_B }] }),
      appliedResponse({ ops: [{ ...results[2], legacy_id: NODE_A }] }),
      { data: appliedResponse({ ops: [results[2]] }) },
    ];
    for (const response of invalidResponses) {
      const port = createCreativeCanvasAgentOpsPort(async () => response);
      const error = await captureAsyncError(() =>
        port.apply({
          projectId: PROJECT_ID,
          assistantMessageId: ASSISTANT_MESSAGE_ID,
          expectedRevision: '7',
          ops: [ops[2]!],
        })
      );
      expect(error instanceof CreativeStudioContractError).toBe(true);
    }
  });

  test('validates typed input including non-finite values before calling the HTTP seam', async () => {
    let calls = 0;
    const port = createCreativeCanvasAgentOpsPort(async () => {
      calls += 1;
      return appliedResponse({ ops: [results[2]] });
    });

    for (const input of [
      {
        projectId: 'legacy-project',
        assistantMessageId: ASSISTANT_MESSAGE_ID,
        expectedRevision: '7',
        ops: [ops[2]!],
      },
      {
        projectId: PROJECT_ID,
        assistantMessageId: 'legacy-message',
        expectedRevision: '7',
        ops: [ops[2]!],
      },
      {
        projectId: PROJECT_ID,
        assistantMessageId: ASSISTANT_MESSAGE_ID,
        expectedRevision: '0',
        ops: [ops[2]!],
      },
      {
        projectId: PROJECT_ID,
        assistantMessageId: ASSISTANT_MESSAGE_ID,
        expectedRevision: '07',
        ops: [ops[2]!],
      },
      {
        projectId: PROJECT_ID,
        assistantMessageId: ASSISTANT_MESSAGE_ID,
        expectedRevision: '9223372036854775807',
        ops: [ops[2]!],
      },
      {
        projectId: PROJECT_ID,
        assistantMessageId: ASSISTANT_MESSAGE_ID,
        expectedRevision: '7',
        ops: [{ ...ops[2]!, x: Number.NaN }],
      },
      {
        projectId: PROJECT_ID,
        assistantMessageId: ASSISTANT_MESSAGE_ID,
        expectedRevision: '7',
        ops: [{ ...ops[3]!, width: Number.POSITIVE_INFINITY }],
      },
    ]) {
      const error = await captureAsyncError(() =>
        port.apply(input as Parameters<typeof port.apply>[0])
      );
      expect(error instanceof CreativeStudioContractError).toBe(true);
      expect((error as CreativeStudioContractError).code).toBe('INVALID_REQUEST');
    }
    expect(calls).toBe(0);
  });
});
