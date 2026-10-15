import { useEffect } from 'react';
import useSWR from 'swr';
import { ipcBridge } from '@/common';
import type { ConversationId } from '@/common/types/ids';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { getConversationActivity } from '@/renderer/pages/conversation/utils/conversationRuntime';

export type SessionCommandActivity = 'idle' | 'busy' | 'unknown';

/** Read the existing Session projection when a host has no live authoritative
 * activity. Local controller inactivity and history never establish idle. */
export function useSessionCommandActivity(conversationId?: ConversationId, suppliedActivity?: SessionCommandActivity): SessionCommandActivity {
  const queryId = suppliedActivity === undefined ? conversationId : undefined;
  const { data, error, isValidating, mutate } = useSWR(
    queryId ? `conversation/${queryId}` : null,
    () => getConversationOrNull(queryId!),
    { shouldRetryOnError: false, revalidateOnMount: true, dedupingInterval: 0 },
  );
  useEffect(() => {
    if (!queryId) return;
    const refresh = (event: { conversation_id: unknown }) => {
      if (String(event.conversation_id) === queryId) void mutate().catch(() => undefined);
    };
    const dispose = [
      ipcBridge.conversation.listChanged.on(refresh),
      ipcBridge.conversation.turnStarted.on(refresh),
      ipcBridge.conversation.turnCompleted.on(refresh),
      ipcBridge.conversation.turnPaused.on(refresh),
      ipcBridge.conversation.reconnected.on(() => { void mutate().catch(() => undefined); }),
    ];
    return () => dispose.forEach(stop => stop());
  }, [queryId, mutate]);

  if (suppliedActivity !== undefined) return suppliedActivity;
  if (!queryId || error || isValidating || data?.id !== queryId) return 'unknown';
  const activity = getConversationActivity(data);
  return activity === 'idle' ? 'idle' : activity === 'running' || activity === 'paused' ? 'busy' : 'unknown';
}
