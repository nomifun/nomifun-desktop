import type { TChatConversation } from '@/common/config/storage';

/** App project membership selects the owning surface without creating another Session. */
export function conversationDestination(conversation: Pick<TChatConversation, 'id' | 'app_project_id'>): string {
  return conversation.app_project_id
    ? `/apps/${encodeURIComponent(conversation.app_project_id)}?session=${encodeURIComponent(conversation.id)}`
    : `/conversation/${conversation.id}`;
}
