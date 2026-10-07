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
  test('re-reads the active workspace when the shared transport requests recovery', () => {
    const response = source<{ conversation_id?: string; type: string; data?: unknown }>();
    const files = source<{ workspace: string }>();
    const turns = source<{ conversation_id: string }>();
    const manual = source<void>();
    const recovery = source<void>();
    const sources = {
      responseStream: response.on, fileUpdates: files.on, turnCompleted: turns.on,
      manual: (listener: () => void) => manual.on(listener),
      reconnected: (listener: () => void) => recovery.on(listener),
    };
    let reads = 0;
    const unsubscribe = subscribeConversationWorkspaceRefresh(sources, 'session-a', 'C:/Work/A', () => { reads += 1; });
    try {
      recovery.emit(undefined);
      expect(reads).toBe(1);
    } finally {
      unsubscribe();
    }
    recovery.emit(undefined);
    expect(reads).toBe(1);
  });

  test('coalesces repeated recovery signals without losing the final reread', async () => {
    const files = source<{ workspace: string }>();
    const recovery = source<void>();
    let reads = 0;
    const unsubscribe = subscribeConversationWorkspaceRefresh({
      responseStream: () => () => {}, turnCompleted: () => () => {}, manual: () => () => {},
      fileUpdates: files.on, reconnected: (listener) => recovery.on(listener),
    }, 'session-a', 'C:/Work/A', () => { reads += 1; });
    try {
      files.emit({ workspace: 'C:/Work/A' });
      recovery.emit(undefined);
      recovery.emit(undefined);
      recovery.emit(undefined);
      expect(reads).toBe(1);
      // The existing refresh coalescing window is 2 seconds.
      await new Promise((resolve) => setTimeout(resolve, 2100));
      expect(reads).toBe(2);
    } finally {
      unsubscribe();
    }
  });

  test('a queued recovery callback cannot refresh a disposed workspace', () => {
    let queued: (() => void) | undefined;
    let detached = false;
    let reads = 0;
    const unsubscribe = subscribeConversationWorkspaceRefresh({
      responseStream: () => () => {}, fileUpdates: () => () => {},
      turnCompleted: () => () => {}, manual: () => () => {},
      reconnected: (listener) => {
        queued = listener;
        return () => { detached = true; };
      },
    }, 'session-a', 'C:/Work/A', () => { reads += 1; });
    expect(queued).toBeDefined();
    unsubscribe();
    expect(detached).toBe(true);
    // The shared transport dispatches from a listener snapshot. It can have
    // captured this callback before another listener caused navigation.
    queued!();
    expect(reads).toBe(0);
  });

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
      reconnected: () => () => {},
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
