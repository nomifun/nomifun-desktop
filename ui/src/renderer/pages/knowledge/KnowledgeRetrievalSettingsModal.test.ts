import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

import type { ProviderId } from '@/common/types/ids';
import {
  retrievalDraftFromWire,
  retrievalWireFromDraft,
} from './KnowledgeRetrievalSettingsModal';

const providerId = '0190f5fe-7c00-7a00-8000-000000000001' as ProviderId;
const modalSource = readFileSync(
  new URL('./KnowledgeRetrievalSettingsModal.tsx', import.meta.url),
  'utf8'
);

describe('Knowledge retrieval settings', () => {
  test('keeps embedding and rerank as independent task-specific stages', () => {
    const draft = retrievalDraftFromWire({
      embedding: { mode: 'local' },
      rerank: { mode: 'remote', provider_id: providerId, model: 'rerank-v3' },
    });

    expect(draft).toEqual({
      embedding: { mode: 'local' },
      rerank: {
        mode: 'remote',
        model: { provider_id: providerId, model: 'rerank-v3' },
      },
    });
    expect(retrievalWireFromDraft(draft)).toEqual({
      embedding: { mode: 'local' },
      rerank: { mode: 'remote', provider_id: providerId, model: 'rerank-v3' },
    });
  });

  test('refuses to serialize a remote stage without a complete model reference', () => {
    let message = '';
    try {
      retrievalWireFromDraft({
        embedding: { mode: 'remote', model: null },
        rerank: { mode: 'local' },
      });
    } catch (error) {
      message = error instanceof Error ? error.message : String(error);
    }
    expect(message).toBe('remote retrieval stage requires a model');
  });

  test('renders both independent stages through the shared task-aware selector', () => {
    expect(modalSource.includes("renderMode('embedding')")).toBe(true);
    expect(modalSource.includes("renderMode('rerank')")).toBe(true);
    expect(modalSource.includes('<TaskModelSelect')).toBe(true);
    expect(modalSource.includes('task={stage}')).toBe(true);
  });
});
