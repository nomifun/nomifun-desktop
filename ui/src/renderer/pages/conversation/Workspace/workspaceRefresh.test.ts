import { describe, expect, test } from 'bun:test';
import { sameWorkspaceForRefresh, subscribeConversationWorkspaceRefresh } from './workspaceRefresh';

const source = <T>() => {
  const listeners = new Set<(event: T) => void>();
  return {
    on: (listener: (event: T) => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    emit: (event: T) => { for (const listener of listeners) listener(event); },
  };
};

describe('workspace file event identity', () => {
  test('matches Windows paths despite separator, case, and trailing slash differences', () => {
    expect(sameWorkspaceForRefresh('C:\\Work\\Project', 'c:/work/project/')).toBe(true);
  });

  test('does not refresh another workspace or conflate case-sensitive paths', () => {
    expect(sameWorkspaceForRefresh('C:\\Work\\One', 'C:\\Work\\Two')).toBe(false);
    expect(sameWorkspaceForRefresh('/tmp/Case', '/tmp/case')).toBe(false);
    expect(sameWorkspaceForRefresh('', '')).toBe(false);
  });

  test('refreshes for published files and settled turns without carrying stale timers into another session', async () => {
    const response = source<{ conversation_id?: string; type: string; data?: unknown }>();
    const files = source<{ workspace: string }>();
    const turns = source<{ conversation_id: string }>();
    const manual = source<void>();
    const sources = {
      responseStream: response.on,
      fileUpdates: files.on,
      turnCompleted: turns.on,
      manual: (listener: () => void) => manual.on(listener),
    };
    let firstRefreshes = 0;
    const unsubscribe = subscribeConversationWorkspaceRefresh(
      sources, 'session-a', 'C:/Work/A', () => { firstRefreshes += 1; }
    );

    response.emit({ conversation_id: 'session-b', type: 'tool_call', data: { status: 'completed' } });
    files.emit({ workspace: 'c:\\work\\a' });
    turns.emit({ conversation_id: 'session-a' });
    expect(firstRefreshes).toBe(1);
    await new Promise((resolve) => setTimeout(resolve, 2100));
    expect(firstRefreshes).toBe(2);
    unsubscribe();
    files.emit({ workspace: 'C:/Work/A' });
    expect(firstRefreshes).toBe(2);

    let secondRefreshes = 0;
    const unsubscribeSecond = subscribeConversationWorkspaceRefresh(
      sources, 'session-b', 'C:/Work/B', () => { secondRefreshes += 1; }
    );
    files.emit({ workspace: 'C:/Work/A' });
    turns.emit({ conversation_id: 'session-a' });
    expect(secondRefreshes).toBe(0);
    files.emit({ workspace: 'C:/Work/B' });
    expect(secondRefreshes).toBe(1);
    unsubscribeSecond();
  });
});
