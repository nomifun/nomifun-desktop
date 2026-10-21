import { useCallback, useEffect, useRef, useState } from 'react';
import useSWR from 'swr';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { SessionInputQueueAction, SessionInputQueueSnapshot } from '@/common/adapter/ipcBridge';
import type { ConversationId } from '@/common/types/ids';
import { uuidv7 } from '@/common/utils';
import { useAddEventListener } from '@/renderer/utils/emitter';

const EMPTY_QUEUE: SessionInputQueueSnapshot = { version: 0, items: [] };

/** A read-only view of Host-owned inputs. Mounting or refreshing never dispatches work. */
export function useSessionInputQueue(conversationId: ConversationId) {
  const snapshotRef = useRef<SessionInputQueueSnapshot>(EMPTY_QUEUE);
  const currentConversation = useRef(conversationId);
  if (currentConversation.current !== conversationId) snapshotRef.current = EMPTY_QUEUE;
  currentConversation.current = conversationId;
  const { data, error, mutate } = useSWR<SessionInputQueueSnapshot>(['session-input-queue', conversationId],
    async (): Promise<SessionInputQueueSnapshot> => {
      const next = await ipcBridge.conversation.inputQueue.invoke({ conversation_id: conversationId });
      const previous = snapshotRef.current;
      return currentConversation.current === conversationId && previous.version > next.version ? previous : next;
    },
    { revalidateOnFocus: true, shouldRetryOnError: false });
  const snapshot = data && data.version >= snapshotRef.current.version ? data : snapshotRef.current;
  if (snapshot.version >= snapshotRef.current.version) snapshotRef.current = snapshot;
  const [activeConversations, setActiveConversations] = useState<Set<ConversationId>>(() => new Set());
  const activeCommands = useRef(new Map<ConversationId, string>());
  const uncertain = useRef(new Map<ConversationId, { signature: string; key: string; version: number }>());
  const changing = activeConversations.has(conversationId);
  const refresh = useCallback(() => { void mutate().catch(() => {}); }, [mutate]);
  useEffect(() => {
    const onEvent = (event: { conversation_id: ConversationId }) => { if (event.conversation_id === conversationId) refresh(); };
    const unsubscribers = [ipcBridge.conversation.userCreated.on(onEvent), ipcBridge.conversation.turnStarted.on(onEvent),
      ipcBridge.conversation.turnCompleted.on(onEvent), ipcBridge.conversation.reconnected.on(refresh)];
    return () => { unsubscribers.forEach(unsubscribe => unsubscribe()); };
  }, [conversationId, refresh]);
  useAddEventListener('chat.history.refresh', refresh, [refresh]);
  useAddEventListener('conversation.history.refresh', id => { if (id === conversationId) refresh(); }, [conversationId, refresh]);
  useAddEventListener('conversation.turn.settled', id => { if (id === conversationId) refresh(); }, [conversationId, refresh]);
  const change = useCallback(async (action: SessionInputQueueAction) => {
    if (activeCommands.current.has(conversationId)) throw new Error('An input queue change is already awaiting acknowledgement');
    const signature = JSON.stringify(action);
    const previous = uncertain.current.get(conversationId);
    const command = previous?.signature === signature
      ? previous : { signature, key: uuidv7(), version: snapshotRef.current.version };
    uncertain.current.set(conversationId, command);
    activeCommands.current.set(conversationId, command.key);
    setActiveConversations(new Set(activeCommands.current.keys()));
    try {
      const next = await ipcBridge.conversation.updateInputQueue.invoke({ conversation_id: conversationId,
        idempotency_key: command.key, expected_version: command.version, action });
      if (currentConversation.current === conversationId) {
        if (next.version >= snapshotRef.current.version) snapshotRef.current = next;
        await mutate(current => !current || next.version >= current.version ? next : current, { revalidate: false });
      }
      if (uncertain.current.get(conversationId)?.key === command.key) uncertain.current.delete(conversationId);
      return next;
    } catch (failure) {
      // A completed client refusal rolled the original writer transaction back.
      // Transport failures keep the exact command identity for reconciliation.
      if (isBackendHttpError(failure) && failure.status >= 400 && failure.status < 500
        && uncertain.current.get(conversationId)?.key === command.key) uncertain.current.delete(conversationId);
      throw failure;
    } finally {
      if (activeCommands.current.get(conversationId) === command.key) activeCommands.current.delete(conversationId);
      setActiveConversations(new Set(activeCommands.current.keys()));
      if (currentConversation.current === conversationId) refresh();
    }
  }, [conversationId, mutate, refresh]);
  return { snapshot, error, loading: !data && !error, changing, refresh, change };
}
