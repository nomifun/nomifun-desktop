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
const modalStyles = readFileSync(
  new URL('./KnowledgeRetrievalSettingsModal.module.css', import.meta.url),
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

  test('separates stage copy from vertically stacked mode buttons', () => {
    expect(modalSource.includes('className={styles.stageHeader}')).toBe(true);
    expect(modalSource.includes("direction='vertical'")).toBe(true);
    expect(modalSource.includes('className={styles.modeGroup}')).toBe(true);
    expect(modalStyles.includes('grid-template-columns: minmax(0, 1fr) 148px')).toBe(true);
    expect(modalStyles.includes('gap: 5px')).toBe(true);
  });

  test('uses compact, high-contrast mode controls', () => {
    expect(modalStyles.includes('font-size: 12px')).toBe(true);
    expect(modalStyles.includes('white-space: nowrap')).toBe(true);
    expect(modalStyles.includes('color: var(--color-text-1) !important')).toBe(true);
    expect(modalStyles.includes('background: rgba(var(--primary-6), 0.1) !important')).toBe(true);
  });

  test('uses outlined stage containers without a filled background', () => {
    expect(modalStyles.includes('border: 1px solid var(--color-border-3)')).toBe(true);
    expect(modalStyles.includes('background: transparent')).toBe(true);
  });

  test('keeps stage titles compact and moderately weighted', () => {
    expect(modalStyles.includes('font-size: 13px')).toBe(true);
    expect(modalStyles.includes('font-weight: 500')).toBe(true);
  });

  test('reduces the modal header, content, and footer vertical spacing', () => {
    expect(modalSource.includes('className={styles.modal}')).toBe(true);
    expect(modalStyles.includes('height: 40px')).toBe(true);
    expect(modalStyles.includes('var(--nomi-modal-block-padding) var(--nomi-modal-inline-padding)')).toBe(true);
  });
});
