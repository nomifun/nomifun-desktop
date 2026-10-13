import { expect, test } from 'bun:test';
import { fromApiSearchResult, type ApiMessageSearchItem } from './searchMapper';

test('message search retains explicit App project ownership without importing extra markers', () => {
  const item: ApiMessageSearchItem = {
    message_id: '0190f5fe-7c00-7a00-8000-000000000021', message_type: 'text', message_created_at: 3, preview_text: 'App history',
    conversation: { conversation_id: '0190f5fe-7c00-7a00-8000-000000000001', name: 'Development', type: 'nomi', status: 'finished',
      pinned: false, created_at: 1, modified_at: 2, extra: { app_project_id: 'untrusted-extra' }, app_project_id: 'owned-app' },
  };
  const result = fromApiSearchResult({ items: [item, { ...item, conversation: { ...item.conversation, app_project_id: null } }],
    total: 2, has_more: false });
  expect(result.items[0].conversation.app_project_id).toBe('owned-app');
  expect(result.items[0].conversation.id).toBe(item.conversation.conversation_id);
  expect(result.items[0].message_id).toBe(item.message_id);
  expect(result.items[1].conversation.app_project_id).toBeUndefined();
});
