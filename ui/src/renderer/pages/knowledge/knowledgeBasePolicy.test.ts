/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { IKnowledgeBase } from '@/common/adapter/ipcBridge';
import { parseKnowledgeBaseId } from '@/common/types/ids';
import { canPurgeKnowledgeBase } from './knowledgeBasePolicy';

let nextId = 0;
function base(overrides: Partial<IKnowledgeBase> = {}): IKnowledgeBase {
  return {
    knowledge_base_id: parseKnowledgeBaseId(`0190f5fe-7c00-7a00-8000-00000000${String(1000 + nextId++).slice(-4)}`),
    name: 'Base',
    description: '',
    root_path: '/knowledge/base',
    managed: true,
    tree_access: 'editable',
    created_at: 1,
    updated_at: 1,
    file_count: 0,
    total_size: 0,
    root_exists: true,
    tags: [],
    kind: 'blank',
    ...overrides,
  };
}

describe('canPurgeKnowledgeBase', () => {
  test('offers purge only for managed bases whose documents live on disk', () => {
    expect(canPurgeKnowledgeBase(base({ managed: true, kind: 'blank' }))).toBe(true);
    expect(canPurgeKnowledgeBase(base({ managed: true, kind: 'web' }))).toBe(true);
    expect(canPurgeKnowledgeBase(base({ managed: true, kind: 'local' }))).toBe(true);
  });

  test('never offers purge for feishu bases — their documents are remote', () => {
    expect(canPurgeKnowledgeBase(base({ managed: true, kind: 'feishu' }))).toBe(false);
  });

  test('never offers purge for external-directory bases', () => {
    expect(canPurgeKnowledgeBase(base({ managed: false, kind: 'local' }))).toBe(false);
    expect(canPurgeKnowledgeBase(base({ managed: false, kind: 'feishu' }))).toBe(false);
  });
});
