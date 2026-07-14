import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { TAgentExecutionChangedEvent } from '@/common/types/agentExecution/agentExecutionEvents';
import type { TAgentExecutionDetail } from '@/common/types/agentExecution/agentExecutionTypes';
import { getConversationOrNull, seedConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useExecutionLive } from './useExecutionLive';
import { useLeadThinking, type LeadThinkingState } from './useLeadThinking';

const RELATION_REFETCH_DEBOUNCE_MS = 120;

export interface ConversationExecutionState {
  executionId: string | null;
  detail: TAgentExecutionDetail | null;
  refetch: () => Promise<void>;
  leadThinking: LeadThinkingState;
  loading: boolean;
}

export function shouldDiscoverExecutionRelation(
  currentExecutionId: string | null,
  event: Pick<TAgentExecutionChangedEvent, 'execution_id' | 'change_kind'>,
): boolean {
  return event.execution_id !== currentExecutionId || event.change_kind === 'deleted';
}

export function useConversationExecution(conversation: TChatConversation | null | undefined): ConversationExecutionState {
  // ConversationExecutionLink is authoritative for every Agent runtime. ACP,
  // Codex, OpenClaw, Nanobot and companion sessions can all delegate through
  // the same process-issued Platform Gateway capability; filtering by
  // conversation type would create an
  // execution that exists in the backend but cannot be observed or controlled.
  const conversationId = conversation?.id;
  const [executionId, setExecutionId] = useState<string | null>(conversation?.linked_execution_id ?? null);
  const relationRequestSequence = useRef(0);
  const relationTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    relationRequestSequence.current += 1;
    setExecutionId(conversation?.linked_execution_id ?? null);
  }, [conversation?.id, conversation?.linked_execution_id]);

  const discoverRelation = useCallback(async () => {
    if (conversationId == null) return;
    const request = ++relationRequestSequence.current;
    try {
      const latest = await getConversationOrNull(conversationId);
      if (request !== relationRequestSequence.current || !latest) return;
      // Keep the route-level SWR value and every other Conversation consumer in
      // sync with the authoritative relation projection returned by the API.
      seedConversationCache(latest);
      setExecutionId(latest.linked_execution_id ?? null);
    } catch (error) {
      console.error('[useConversationExecution] Failed to discover execution relation:', error);
    }
  }, [conversationId]);

  useEffect(() => {
    if (conversationId == null) return;
    const unsubscribe = ipcBridge.agentExecution.events.changed.on((event) => {
      if (!shouldDiscoverExecutionRelation(executionId, event)) return;
      if (relationTimer.current !== null) clearTimeout(relationTimer.current);
      relationTimer.current = setTimeout(() => {
        relationTimer.current = null;
        void discoverRelation();
      }, RELATION_REFETCH_DEBOUNCE_MS);
    });
    return () => {
      unsubscribe();
      relationRequestSequence.current += 1;
      if (relationTimer.current !== null) clearTimeout(relationTimer.current);
      relationTimer.current = null;
    };
  }, [conversationId, discoverRelation, executionId]);

  const { detail, loading, refetch } = useExecutionLive(executionId ?? undefined);
  const leadThinking = useLeadThinking(executionId);
  return { executionId, detail, refetch, leadThinking, loading };
}
