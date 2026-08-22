/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeStudioAgentMessage } from '../../../agent';
import { CREATIVE_CANVAS_AGENT_ARTIFACT_KIND } from './artifacts';
import { projectCreativeCanvasAgentProposals } from './proposalProjection';

const READY_ID = '0190f5fe-7c00-7a00-8000-000000000931';
const APPLIED_ID = '0190f5fe-7c00-7a00-8000-000000000932';
const INVALID_ID = '0190f5fe-7c00-7a00-8000-000000000933';
const NODE_ID = '0190f5fe-7c00-7a00-8000-000000000934';

const artifact = (summary: string): string => `\`\`\`json
${JSON.stringify({
  kind: CREATIVE_CANVAS_AGENT_ARTIFACT_KIND,
  summary,
  ops: [{ type: 'move_node', node_id: NODE_ID, x: 10, y: 20 }],
})}
\`\`\``;

describe('Creative Canvas proposal projection', () => {
  test('durable receipt authority wins over a stale in-memory failure after remount', () => {
    const messages: CreativeStudioAgentMessage[] = [
      { id: READY_ID, role: 'assistant', status: 'complete', text: artifact('待应用') },
      { id: APPLIED_ID, role: 'assistant', status: 'complete', text: artifact('已应用') },
      {
        id: INVALID_ID,
        role: 'assistant',
        status: 'complete',
        text: `\`\`\`json
{"kind":"${CREATIVE_CANVAS_AGENT_ARTIFACT_KIND}","summary":
\`\`\``,
      },
      { id: 'ordinary', role: 'assistant', status: 'complete', text: '普通建议' },
      { id: 'running', role: 'assistant', status: 'running', text: artifact('未完成') },
    ];

    const projected = projectCreativeCanvasAgentProposals(
      messages,
      {
        [APPLIED_ID]: {
          state: 'failed',
          errorMessage: '旧页面曾丢失响应',
        },
      },
      [APPLIED_ID]
    );

    expect(projected.proposals).toEqual([
      {
        messageId: READY_ID,
        summary: '待应用',
        opCount: 1,
        state: 'ready',
      },
      {
        messageId: APPLIED_ID,
        summary: '已应用',
        opCount: 1,
        state: 'applied',
      },
      {
        messageId: INVALID_ID,
        summary: 'Agent 返回的画布提案格式无效',
        opCount: 0,
        state: 'invalid',
        errorMessage: '该提案未通过严格合同校验，不能应用到画布。',
      },
    ]);
    expect(projected.artifacts.has(READY_ID)).toBe(true);
    expect(projected.artifacts.has(APPLIED_ID)).toBe(true);
    expect(projected.artifacts.has(INVALID_ID)).toBe(false);
  });
});
