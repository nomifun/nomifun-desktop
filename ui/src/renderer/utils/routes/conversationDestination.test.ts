import { expect, test } from 'bun:test';
import { parseCompanionId, parseConversationId } from '@/common/types/ids';
import { conversationDestination } from './conversationDestination';

test('the same canonical Session opens through its owning App or ordinary conversation surface', () => {
  const id = parseConversationId('0190f5fe-7c00-7a00-8000-000000000001');
  expect(conversationDestination({ id })).toBe(`/conversation/${id}`);
  expect(conversationDestination({ id, app_project_id: 'owned-app' })).toBe(`/apps/owned-app?session=${id}`);
  expect(conversationDestination({ id, app_project_id: 'project/name' })).toBe(`/apps/project%2Fname?session=${id}`);
});

test('companion navigation preserves the exact Session for either companion role', () => {
  const companionId = parseCompanionId('0190f5fe-7c00-7a00-8000-000000000010');
  const firstSession = parseConversationId('0190f5fe-7c00-7a00-8000-000000000001');
  const robotSession = parseConversationId('0190f5fe-7c00-7a00-8000-000000000002');
  const extra = { workspace: '', companion_id: companionId, companion_role: 'companion' as const };
  const robotExtra = { ...extra, companion_role: 'robot' as const };

  expect(conversationDestination({ id: firstSession, extra })).toBe(
    `/nomi?companion=${companionId}&session=${firstSession}&mode=cohabit`,
  );
  expect(conversationDestination({ id: robotSession, extra: robotExtra })).toBe(
    `/nomi?companion=${companionId}&session=${robotSession}&mode=cohabit`,
  );
});

test('a retired companion marker cannot claim an ordinary Session', () => {
  const id = parseConversationId('0190f5fe-7c00-7a00-8000-000000000001');
  const extra = { workspace: '', companion_session: true };
  expect(conversationDestination({ id, extra })).toBe(`/conversation/${id}`);
});
