import type { ConversationId } from '@/common/types/ids';
import { ipcBridge } from '@/common';

const warmupByConversation = new Map<ConversationId, Promise<void>>();

export function warmupConversation(conversation_id: ConversationId): Promise<void> {
  const existing = warmupByConversation.get(conversation_id);
  if (existing) return existing;

  const promise = ipcBridge.conversation.warmup.invoke({ conversation_id }).finally(() => {
    warmupByConversation.delete(conversation_id);
  });
  warmupByConversation.set(conversation_id, promise);
  return promise;
}
