import { useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import type { ConversationId } from '@/common/types/ids';
import type { TaskPlanSnapshot } from '@/common/protocolBindings/TaskPlanSnapshot';
import { addEventListener } from '@/renderer/utils/emitter';

/** Latest-state progress has no dependency on transcript loading or paging.
 * Notifications request authoritative reads, so delayed events cannot restore
 * an old turn. Reads are serialized and notifications during a read coalesce
 * into one follow-up. Polling repairs dropped notifications; scope cleanup and
 * sequence checks fence navigation and stale responses. */
export function useConversationTaskPlan(conversationId: ConversationId, running = false) {
  const [snapshot, setSnapshot] = useState<TaskPlanSnapshot | null>(null);
  const runningRef = useRef(running);
  runningRef.current = running;
  const refreshRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let disposed = false;
    let reading = false;
    let requested = false;
    let latest: TaskPlanSnapshot | null = null;
    let poll: ReturnType<typeof setTimeout> | undefined;
    let cancelRead: (() => void) | undefined;
    setSnapshot(null);

    const refresh = () => {
      if (disposed) return;
      requested = true;
      if (reading) return;
      reading = true;
      clearTimeout(poll);
      void (async () => {
        let failed = false;
        while (requested && !disposed) {
          requested = false;
          let timeout: ReturnType<typeof setTimeout> | undefined;
          try {
            const next = await Promise.race([
              ipcBridge.conversation.taskPlan.invoke({ conversation_id: conversationId }),
              new Promise<never>((_, reject) => {
                cancelRead = () => reject(new Error('Task plan read superseded'));
                timeout = setTimeout(() => reject(new Error('Task plan read timed out')), 4_000);
              }),
            ]);
            if (disposed) break;
            if (next.conversation_id !== conversationId) throw new Error('Task plan belongs to another conversation');
            if (!latest || next.sequence >= latest.sequence) {
              latest = next;
              setSnapshot(next);
            }
            failed = false;
          } catch (error) {
            failed = true;
            if (!disposed) console.warn('[conversation-task-plan] Failed to read progress:', error);
          } finally {
            clearTimeout(timeout);
            cancelRead = undefined;
          }
        }
        reading = false;
        if (!disposed) {
          const active = runningRef.current || latest?.turn_status === 'running';
          poll = setTimeout(refresh, failed ? 3_000 : active ? 4_000 : 30_000);
        }
      })();
    };
    refreshRef.current = refresh;
    const offStream = ipcBridge.conversation.responseStream.on((event) => {
      if (event.conversation_id === conversationId &&
          ['task_plan_changed', 'start', 'finish', 'error'].includes(event.type)) refresh();
    });
    const offStarted = ipcBridge.conversation.turnStarted.on((event) => {
      if (event.conversation_id === conversationId) refresh();
    });
    const offCompleted = ipcBridge.conversation.turnCompleted.on((event) => {
      if (event.conversation_id === conversationId) refresh();
    });
    const offReconnect = ipcBridge.conversation.reconnected.on(refresh);
    const offSettled = addEventListener('conversation.turn.settled', (id) => {
      if (id === conversationId) refresh();
    });
    window.addEventListener('focus', refresh);
    refresh();
    return () => {
      disposed = true;
      clearTimeout(poll);
      cancelRead?.();
      refreshRef.current = null;
      offStream(); offStarted(); offCompleted(); offReconnect(); offSettled();
      window.removeEventListener('focus', refresh);
    };
  }, [conversationId]);

  useEffect(() => {
    if (running) refreshRef.current?.();
  }, [running]);

  return snapshot?.conversation_id === conversationId ? snapshot : null;
}
