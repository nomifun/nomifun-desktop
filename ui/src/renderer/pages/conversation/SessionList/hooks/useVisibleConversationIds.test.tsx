import '../../../../../../test/setup-dom.ts';
import { cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId } from '@/common/types/ids';
import * as History from '@/renderer/hooks/context/ConversationHistoryContext';
import * as Terminals from '@/renderer/pages/terminal/useTerminalSessions';
import { useVisibleConversationIds } from './useVisibleConversationIds';

afterEach(() => { cleanup(); mock.restore(); });
test('conversation cycling excludes application members while preserving ordinary project ordering', () => {
  const conversations = [1, 2, 3].map((n) => ({ id: parseConversationId(`0190f5fe-7c00-7a00-8000-${String(n).padStart(12, '0')}`),
    type: 'nomi', name: String(n), created_at: 1, modified_at: n, extra: { workspace: 'C:/shared/project', custom_workspace: true },
    ...(n === 2 ? { app_project_id: 'owned-app' } : {}) } as TChatConversation));
  spyOn(History, 'useConversationHistoryContext').mockReturnValue({ conversations } as ReturnType<typeof History.useConversationHistoryContext>);
  spyOn(Terminals, 'useTerminalSessions').mockReturnValue({ sessions: [], loading: false, refresh: async () => {}, removeSession: async () => {} });
  const hook = renderHook(useVisibleConversationIds);
  expect(hook.result.current).toEqual([conversations[2].id, conversations[0].id]);
});
