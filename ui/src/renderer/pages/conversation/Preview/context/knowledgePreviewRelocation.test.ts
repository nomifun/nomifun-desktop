import { describe, expect, test } from 'bun:test';

import type { KnowledgeBaseId } from '@/common/types/ids';

import type { PreviewTab } from './PreviewContext';
import { relocateKnowledgePreviewTabs } from './knowledgePreviewRelocation';

const knowledgeBaseId = '0190f5fe-7c00-7a00-8000-000000000001' as KnowledgeBaseId;

describe('knowledge preview relocation', () => {
  test('rebinds a moved folder subtree without discarding dirty content', () => {
    const tab: PreviewTab = {
      id: 'tab-1',
      content: '# unsaved',
      originalContent: '# saved',
      content_type: 'markdown',
      title: '产品库 / topic.md',
      isDirty: true,
      metadata: {
        title: '产品库 / topic.md',
        file_name: 'topic.md',
        file_path: '/vault/raw/topic.md',
        workspace: '/vault',
        knowledge_resource: {
          kind: 'knowledge-document',
          knowledge_base_id: knowledgeBaseId,
          rel_path: 'raw/topic.md',
        },
      },
    };

    const result = relocateKnowledgePreviewTabs([tab], {
      knowledge_base_id: knowledgeBaseId,
      old_prefix: 'raw',
      new_prefix: 'archive/raw',
    });

    expect(result[0]).toMatchObject({
      id: 'tab-1',
      content: '# unsaved',
      originalContent: '# saved',
      isDirty: true,
      metadata: {
        file_path: '/vault/archive/raw/topic.md',
        knowledge_resource: { rel_path: 'archive/raw/topic.md' },
      },
    });
  });

  test('preserves referential identity when no tab belongs to the moved subtree', () => {
    const tabs: PreviewTab[] = [];
    expect(
      relocateKnowledgePreviewTabs(tabs, {
        knowledge_base_id: knowledgeBaseId,
        old_prefix: 'raw',
        new_prefix: 'archive/raw',
      }),
    ).toBe(tabs);
  });
});
