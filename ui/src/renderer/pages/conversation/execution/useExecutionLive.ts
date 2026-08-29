import { ipcBridge } from '@/common';
import type { TAgentExecutionDetail } from '@/common/types/agentExecution/agentExecutionTypes';
import { useCallback, useEffect, useRef, useState } from 'react';
import type { ExecutionId } from '@/common/types/ids';

const EVENT_REFETCH_DEBOUNCE_MS = 180;
const ACTIVE_EXECUTION_POLL_MS = 2_000;

const ACTIVE_EXECUTION_STATUSES = new Set([
  'planning',
  'running',
  'waiting_input',
]);

export function useExecutionLive(executionId: ExecutionId | undefined): {
  detail: TAgentExecutionDetail | null;
  loading: boolean;
  refetch: () => Promise<void>;
} {
  const [detail, setDetail] = useState<TAgentExecutionDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const requestSequence = useRef(0);
  const appliedEventSequence = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const detailRef = useRef<TAgentExecutionDetail | null>(null);
  const requestInFlight = useRef(false);
  const pendingRefetch = useRef(false);
  const latestRefetch = useRef<() => Promise<void>>(() => Promise.resolve());

  const refetch = useCallback(async () => {
    if (!executionId) {
      requestSequence.current += 1;
      pendingRefetch.current = false;
      detailRef.current = null;
      setDetail(null);
      setLoading(false);
      return;
    }
    if (requestInFlight.current) {
      // A conversation/execution switch can happen while the previous GET is
      // still in flight. Queue one refresh for the newest callback instead of
      // silently dropping the initial load for the replacement execution.
      pendingRefetch.current = true;
      return;
    }
    const sequence = ++requestSequence.current;
    requestInFlight.current = true;
    setLoading(true);
    try {
      const next = await ipcBridge.agentExecution.get.invoke({
        execution_id: executionId,
      });
      if (sequence === requestSequence.current) {
        appliedEventSequence.current = next?.execution.event_sequence ?? 0;
        detailRef.current = next ?? null;
        setDetail(next ?? null);
      }
    } catch (error) {
      console.error('[useExecutionLive] Failed to fetch execution detail:', error);
      if (sequence === requestSequence.current) setDetail(null);
    } finally {
      if (sequence === requestSequence.current) setLoading(false);
      requestInFlight.current = false;
      if (pendingRefetch.current) {
        pendingRefetch.current = false;
        queueMicrotask(() => void latestRefetch.current());
      }
    }
  }, [executionId]);

  useEffect(() => {
    latestRefetch.current = refetch;
  }, [refetch]);

  useEffect(() => {
    if (!executionId) return;
    // WebSocket events are the fast path, not the sole source of truth. A
    // disconnected/overloaded socket can lose a committed outbox notification
    // without changing the durable execution row. Poll only while the
    // aggregate is active so the canvas keeps flowing even when no event
    // arrives, while terminal executions remain completely event-driven.
    const poll = window.setInterval(() => {
      const status = detailRef.current?.execution.status;
      if (status && !ACTIVE_EXECUTION_STATUSES.has(status)) return;
      void refetch();
    }, ACTIVE_EXECUTION_POLL_MS);
    return () => window.clearInterval(poll);
  }, [executionId, refetch]);

  useEffect(() => {
    requestSequence.current += 1;
    appliedEventSequence.current = 0;
    detailRef.current = null;
    if (requestInFlight.current) {
      pendingRefetch.current = true;
    } else {
      setDetail(null);
    }
    void refetch();
  }, [refetch]);

  useEffect(() => {
    if (!executionId) return;
    const unsubscribe = ipcBridge.agentExecution.events.changed.on((event) => {
      if (event.execution_id !== executionId) return;
      if (event.sequence <= appliedEventSequence.current) return;
      if (timer.current !== null) clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        timer.current = null;
        void refetch();
      }, EVENT_REFETCH_DEBOUNCE_MS);
    });
    // The realtime bridge deliberately marks outbox rows published even when
    // this renderer was disconnected. A reconnect/resync signal therefore
    // must always refetch the authoritative execution snapshot; relying only
    // on a missed `agentExecution.changed` event leaves task/attempt states
    // frozen until the user manually refreshes the page.
    const unsubscribeReconnect = ipcBridge.conversation.reconnected.on(() => {
      if (timer.current !== null) clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        timer.current = null;
        void refetch();
      }, EVENT_REFETCH_DEBOUNCE_MS);
    });
    return () => {
      unsubscribe();
      unsubscribeReconnect();
      if (timer.current !== null) clearTimeout(timer.current);
      timer.current = null;
    };
  }, [executionId, refetch]);

  return { detail, loading, refetch };
}
