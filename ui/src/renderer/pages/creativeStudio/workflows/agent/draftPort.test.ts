/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { parseProviderId } from '@/common/types/ids';

import {
  CREATIVE_STUDIO_WORKFLOW_DRAFTS_ENDPOINT,
  createWorkflowDraftPort,
} from './draftPort';

const PROVIDER_ID = parseProviderId('0190f5fe-7c00-7a00-8000-000000000321');

const captureError = async (operation: () => Promise<unknown>): Promise<unknown> => {
  try {
    await operation();
    return null;
  } catch (error) {
    return error;
  }
};

describe('Workflow draft one-shot port', () => {
  test('posts one exact model request and parses the strict response envelope', async () => {
    const calls: unknown[] = [];
    const port = createWorkflowDraftPort(async (method, path, body) => {
      calls.push({ method, path, body });
      return { text: '```json\n{"kind":"nomifun.creative-studio.workflow-draft/v1"}\n```' };
    });

    const result = await port.draft({
      providerId: PROVIDER_ID,
      model: 'nomi-chat',
      prompt: '创建电商主图工作流',
    });

    expect(calls).toEqual([
      {
        method: 'POST',
        path: CREATIVE_STUDIO_WORKFLOW_DRAFTS_ENDPOINT,
        body: {
          prompt: '创建电商主图工作流',
          model: { providerId: PROVIDER_ID, model: 'nomi-chat' },
        },
      },
    ]);
    expect(result.text.includes('workflow-draft/v1')).toBe(true);
  });

  test('rejects non-canonical input before transport', async () => {
    let calls = 0;
    const port = createWorkflowDraftPort(async () => {
      calls += 1;
      return { text: 'unused' };
    });

    const error = await captureError(() =>
      port.draft({ providerId: PROVIDER_ID, model: ' model ', prompt: 'draft' })
    );
    expect(error instanceof Error).toBe(true);
    expect(calls).toBe(0);
  });

  test('fails closed for extra, missing, blank, or oversized response fields', async () => {
    for (const response of [
      {},
      { text: '' },
      { text: 'ok', extra: true },
      { text: 'x'.repeat(262_157) },
      { text: '你'.repeat(100_000) },
    ]) {
      const port = createWorkflowDraftPort(async () => response);
      const error = await captureError(() =>
        port.draft({ providerId: PROVIDER_ID, model: 'nomi-chat', prompt: 'draft' })
      );
      expect(error instanceof Error).toBe(true);
    }
  });

  test('accepts the exact JSON-plus-fence response boundary', async () => {
    const text = 'x'.repeat(262_156);
    const port = createWorkflowDraftPort(async () => ({ text }));

    const result = await port.draft({
      providerId: PROVIDER_ID,
      model: 'nomi-chat',
      prompt: 'draft',
    });
    expect(result.text.length).toBe(262_156);
  });
});
