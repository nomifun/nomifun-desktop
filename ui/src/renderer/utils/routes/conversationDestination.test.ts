import { expect, test } from 'bun:test';
import { parseConversationId } from '@/common/types/ids';
import { conversationDestination } from './conversationDestination';

test('the same canonical Session opens through its owning App or ordinary conversation surface', () => {
  const id = parseConversationId('0190f5fe-7c00-7a00-8000-000000000001');
  expect(conversationDestination({ id })).toBe(`/conversation/${id}`);
  expect(conversationDestination({ id, app_project_id: 'owned-app' })).toBe(`/apps/owned-app?session=${id}`);
  expect(conversationDestination({ id, app_project_id: 'project/name' })).toBe(`/apps/project%2Fname?session=${id}`);
});
