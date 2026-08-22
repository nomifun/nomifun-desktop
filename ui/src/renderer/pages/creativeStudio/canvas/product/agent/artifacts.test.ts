/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { CreativeStudioContractError } from '../../../domain';
import {
  CREATIVE_CANVAS_AGENT_ARTIFACT_KIND,
  MAX_CREATIVE_CANVAS_AGENT_ARTIFACT_JSON_BYTES,
  parseCreativeCanvasAgentArtifact,
  type CreativeCanvasAgentOp,
} from './artifacts';

const NODE_A = '0190f5fe-7c00-7a00-8000-000000000801';
const NODE_B = '0190f5fe-7c00-7a00-8000-000000000802';
const CONNECTION = '0190f5fe-7c00-7a00-9000-000000000803';

const addNodeOp: Extract<CreativeCanvasAgentOp, { type: 'add_node' }> = {
  type: 'add_node',
  node_type: 'text',
  x: -12.5,
  y: 24,
  width: 320,
  height: 180,
  group_id: null,
  data: {
    text: '# 标题',
    format: 'markdown',
    fontSize: 32,
    textAlign: 'center',
  },
};

const allAllowedOps: CreativeCanvasAgentOp[] = [
  addNodeOp,
  {
    type: 'update_node_data',
    node_id: NODE_A,
    patch: { text: '更新文案', fontSize: 20 },
  },
  { type: 'move_node', node_id: NODE_A, x: 100, y: -50 },
  { type: 'resize_node', node_id: NODE_A, width: 400, height: 220 },
  {
    type: 'connect',
    source_node_id: NODE_A,
    target_node_id: NODE_B,
    source_handle: 'output',
    target_handle: null,
  },
  { type: 'disconnect', connection_id: CONNECTION },
];

const artifact = (ops: unknown = allAllowedOps, extra: Record<string, unknown> = {}) => ({
  kind: CREATIVE_CANVAS_AGENT_ARTIFACT_KIND,
  summary: '新增并整理文案节点',
  ops,
  ...extra,
});

const fence = (value: unknown): string => `\`\`\`json\n${JSON.stringify(value)}\n\`\`\``;

const expectContractFailure = (text: string): CreativeStudioContractError => {
  try {
    parseCreativeCanvasAgentArtifact(text);
    throw new Error('Expected canvas artifact contract failure');
  } catch (error) {
    expect(error instanceof CreativeStudioContractError).toBe(true);
    return error as CreativeStudioContractError;
  }
};

describe('Creative Canvas Agent artifact parser', () => {
  test('accepts a conversational prefix followed by one final JSON artifact fence', () => {
    expect(parseCreativeCanvasAgentArtifact(`我准备了以下安全变更：\n${fence(artifact())}`)).toEqual(
      artifact()
    );
    expect(
      parseCreativeCanvasAgentArtifact(`我准备了以下安全变更：\n${fence(artifact())}\n \t`)
    ).toEqual(artifact());
  });

  test('accepts every operation in the first non-destructive allowlist', () => {
    const parsed = parseCreativeCanvasAgentArtifact(fence(artifact()));
    expect(parsed?.ops).toEqual(allAllowedOps);
  });

  test('leaves ordinary prose and artifacts owned by other products untouched', () => {
    expect(parseCreativeCanvasAgentArtifact('这里是普通建议，没有结构化变更。')).toBeNull();
    expect(
      parseCreativeCanvasAgentArtifact(
        fence({ kind: 'nomifun.other-product/v1', summary: 'other', ops: [] })
      )
    ).toBeNull();
  });

  test('rejects malformed target JSON, extra fences and a target fence that is not final', () => {
    expectContractFailure(
      `\`\`\`json\n{"kind":"${CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}","summary":\n\`\`\``
    );
    expectContractFailure(`\`\`\`text\nplan\n\`\`\`\n${fence(artifact())}`);
    expectContractFailure(`${fence(artifact())}\ntrailing assistant prose`);
  });

  test('rejects duplicate decoded keys instead of accepting JSON.parse last-write semantics', () => {
    expectContractFailure(
      `\`\`\`json\n{"kind":"${CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}","summary":"first","\\u0073ummary":"second","ops":[${JSON.stringify(allAllowedOps[0])}]}\n\`\`\``
    );
    expectContractFailure(
      `\`\`\`json\n{"kind":"${CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}","summary":"nested","ops":[{"type":"move_node","node_id":"${NODE_A}","x":1,"x":2,"y":3}]}\n\`\`\``
    );
  });

  test('enforces exact top-level, operation, text-data and patch fields', () => {
    expectContractFailure(fence(artifact(allAllowedOps, { version: 1 })));
    expectContractFailure(
      fence(
        artifact([
          {
            ...allAllowedOps[0],
            id: NODE_A,
            zIndex: 10,
            locked: false,
          },
        ])
      )
    );
    expectContractFailure(
      fence(
        artifact([
          {
            ...allAllowedOps[0],
            data: { ...addNodeOp.data, providerId: NODE_A },
          },
        ])
      )
    );
    expectContractFailure(
      fence(
        artifact([
          { type: 'update_node_data', node_id: NODE_A, patch: { status: 'running' } },
        ])
      )
    );
    expectContractFailure(
      fence(artifact([{ type: 'update_node_data', node_id: NODE_A, patch: {} }]))
    );
  });

  test('rejects deletion, media adds and unknown operation discriminants', () => {
    expectContractFailure(fence(artifact([{ type: 'delete_node', node_id: NODE_A }])));
    expectContractFailure(
      fence(
        artifact([
          {
            type: 'add_node',
            node_type: 'image',
            x: 0,
            y: 0,
            data: { assetId: null },
          },
        ])
      )
    );
    expectContractFailure(fence(artifact([{ type: 'run_generation', node_id: NODE_A }])));
  });

  test('enforces bounded canonical values and operation batch limits', () => {
    expectContractFailure(fence({ ...artifact(), summary: ' padded ' }));
    expectContractFailure(fence({ ...artifact(), summary: 'x'.repeat(501) }));
    expectContractFailure(fence(artifact([])));
    expectContractFailure(fence(artifact(Array.from({ length: 65 }, () => allAllowedOps[2]))));
    expectContractFailure(
      fence(artifact([{ type: 'move_node', node_id: 'legacy-node', x: 0, y: 0 }]))
    );
    expectContractFailure(
      fence(
        artifact([
          { ...addNodeOp, width: 0 },
        ])
      )
    );
    expectContractFailure(
      fence(
        artifact([
          { ...addNodeOp, width: 0.5 },
        ])
      )
    );
    expectContractFailure(
      fence(
        artifact([
          {
            ...addNodeOp,
            data: { ...addNodeOp.data, text: 'x'.repeat(20_001) },
          },
        ])
      )
    );
    expectContractFailure(
      fence(
        artifact([
          {
            type: 'connect',
            source_node_id: NODE_A,
            target_node_id: NODE_B,
            source_handle: ' padded ',
          },
        ])
      )
    );
  });

  test('rejects oversized artifact JSON before materializing its value graph', () => {
    const oversized = `\`\`\`json\n{"kind":"${CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}","summary":"safe","ops":[],"padding":"${'x'.repeat(MAX_CREATIVE_CANVAS_AGENT_ARTIFACT_JSON_BYTES)}"}\n\`\`\``;
    const error = expectContractFailure(oversized);
    expect(error.expected.includes('UTF-8 bytes')).toBe(true);
  });

  test('rejects browser-only strings with unpaired UTF-16 surrogates', () => {
    expectContractFailure(fence({ ...artifact(), summary: `bad\ud800summary` }));
    expectContractFailure(
      fence(
        artifact([
          {
            ...addNodeOp,
            data: { ...addNodeOp.data, text: `bad\udc00text` },
          },
        ])
      )
    );
  });
});
