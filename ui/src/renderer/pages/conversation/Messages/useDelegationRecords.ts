import { useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import { database } from '@/common/adapter/ipcBridge';
import type { ConversationId, ExecutionId, MessageId } from '@/common/types/ids';
import type { DelegationRecord } from '@/common/chat/delegationRecords';
import { addEventListener } from '@/renderer/utils/emitter';
import { compareDelegationRecords } from './delegationProcessModel';

type RecordWindow = { roots: string; records: Map<string, DelegationRecord>; cursor?: string };

/** Reads only loaded requests. More continues opaque snapshots; refresh replays loaded depth. */
export function useDelegationRecords(conversationId: ConversationId | undefined, roots: MessageId[], running: boolean, executionId?: ExecutionId | null) {
  const scope = [...new Set(roots)].sort().join(',');
  const depth = useRef({ scope: '', pages: 1 });
  const [revision, setRevision] = useState(0);
  const live = useRef({ running, executionId });
  live.current = { running, executionId };
  const refreshCurrent = useRef<() => void>(() => {});
  const moreCurrent = useRef<() => void>(() => {});
  const lastLifecycle = useRef({ running, executionId });
  const [state, setState] = useState<{ conversationId: ConversationId; scope: string; records: DelegationRecord[]; hasMore: boolean; failed: boolean; loading: boolean }>();
  useEffect(() => {
    if (!conversationId || !scope) return;
    let disposed = false;
    let fetching = false;
    let pendingRefresh = false;
    let pendingMore = false;
    let knownExecutions = new Set<ExecutionId>();
    let changed: (() => void) | undefined;
    let unfinished = false;
    let windows: RecordWindow[] = [];
    let loadedDepth = depth.current.scope === scope ? depth.current.pages : 1;
    depth.current = { scope, pages: loadedDepth };
    const rootIds = scope.split(',');
    const validate = (record: DelegationRecord) => {
      if (record.source.agent_session_id !== conversationId || record.anchor.agent_session_id !== conversationId
        || !rootIds.includes(record.source.source_message_id)) throw new Error('Delegation source did not match this request window');
    };
    const observeExecutions = () => {
      if (changed || (!knownExecutions.size && !live.current.executionId)) return;
      changed = ipcBridge.agentExecution.events.changed.on(event => {
        if (knownExecutions.has(event.execution_id) || event.execution_id === live.current.executionId) void read('refresh');
      });
    };
    const publish = () => {
      const records = new Map(windows.flatMap(window => [...window.records]));
      knownExecutions = new Set([...records.values()].map(record => record.execution_id));
      observeExecutions();
      const settled = new Set([...records.values()].filter(record => record.kind !== 'delegated').map(record => record.step_id));
      unfinished = [...records.values()].some(record => record.kind === 'delegated' && !settled.has(record.step_id));
      setState({ conversationId, scope, records: [...records.values()].sort(compareDelegationRecords),
        hasMore: windows.some(window => Boolean(window.cursor)), failed: false, loading: false });
    };
    const read = async (mode: 'refresh' | 'more') => {
      if (fetching) { if (mode === 'refresh') pendingRefresh = true; else pendingMore = true; return; }
      if (mode === 'more' && !windows.some(window => window.cursor)) return;
      fetching = true;
      setState(previous => previous?.conversationId === conversationId && previous.scope === scope ? { ...previous, loading: true }
        : { conversationId, scope, records: [], hasMore: false, failed: false, loading: true });
      try {
        if (mode === 'more') {
          // Preserve this requested depth even on failure, so Retry can reconcile the whole window.
          depth.current = { scope, pages: loadedDepth + 1 };
          const next: RecordWindow[] = [];
          for (const window of windows) {
            if (!window.cursor) { next.push(window); continue; }
            const page = await database.getDelegationRecords.invoke({ conversation_id: conversationId,
              source_message_ids: window.roots, cursor: window.cursor, limit: 200 });
            if (disposed) return;
            if (page.next_cursor === window.cursor) throw new Error('Delegation cursor did not advance');
            const records = new Map(window.records);
            for (const record of page.records) { validate(record); records.set(record.id, record); }
            next.push({ ...window, records, cursor: page.next_cursor ?? undefined });
          }
          windows = next; loadedDepth += 1;
        } else {
          const next: RecordWindow[] = [];
          for (let start = 0; start < rootIds.length; start += 100) {
            const roots = rootIds.slice(start, start + 100).join(',');
            let cursor: string | undefined;
            const records = new Map<string, DelegationRecord>();
            for (let pageIndex = 0; pageIndex < loadedDepth; pageIndex += 1) {
              const page = await database.getDelegationRecords.invoke({ conversation_id: conversationId, source_message_ids: roots,
                ...(cursor ? { cursor } : {}), limit: 200 });
              if (disposed) return;
              if (cursor && page.next_cursor === cursor) throw new Error('Delegation cursor did not advance');
              for (const record of page.records) { validate(record); records.set(record.id, record); }
              cursor = page.next_cursor ?? undefined;
              if (!cursor) break;
            }
            next.push({ roots, records, cursor });
          }
          windows = next;
        }
        depth.current = { scope, pages: loadedDepth };
        publish();
      } catch {
        if (!disposed) setState(previous => previous?.conversationId === conversationId && previous.scope === scope
          ? { ...previous, failed: true, loading: false } : { conversationId, scope, records: [], hasMore: false, failed: true, loading: false });
      } finally {
        fetching = false;
        if (!disposed && pendingRefresh) { pendingRefresh = false; void read('refresh'); }
        else if (!disposed && pendingMore) { pendingMore = false; void read('more'); }
      }
    };
    refreshCurrent.current = () => void read('refresh');
    moreCurrent.current = () => void read('more');
    void read('refresh');
    observeExecutions();
    const settled = addEventListener('conversation.turn.settled', id => { if (id === conversationId) void read('refresh'); });
    const history = addEventListener('conversation.history.refresh', id => { if (id === conversationId) void read('refresh'); });
    const reconnect = ipcBridge.conversation.reconnected.on(() => void read('refresh'));
    const poll = window.setInterval(() => { if (live.current.running || unfinished) void read('refresh'); }, 2_000);
    return () => { disposed = true; refreshCurrent.current = () => {}; moreCurrent.current = () => {};
      changed?.(); settled(); history(); reconnect(); window.clearInterval(poll); };
  }, [conversationId, scope, revision]);
  useEffect(() => {
    const previous = lastLifecycle.current;
    lastLifecycle.current = { running, executionId };
    if ((previous.running && !running) || previous.executionId !== executionId) refreshCurrent.current();
  }, [running, executionId]);
  const current = state && state.conversationId === conversationId && state.scope === scope ? state : undefined;
  return { records: current?.records ?? [], hasMore: current?.hasMore ?? false, failed: current?.failed ?? false, loading: current?.loading ?? false,
    loadMore: () => moreCurrent.current(), retry: () => setRevision(value => value + 1) };
}
