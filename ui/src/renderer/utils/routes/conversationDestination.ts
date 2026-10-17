import type { TChatConversation } from '@/common/config/storage';

/** Host-resolved product ownership selects the surface and preserves the exact Session. */
export function conversationDestination(
  conversation: Pick<TChatConversation, 'id' | 'app_project_id'> & Partial<Pick<TChatConversation, 'extra'>>,
): string {
  if (conversation.app_project_id) {
    return `/apps/${encodeURIComponent(conversation.app_project_id)}?session=${encodeURIComponent(conversation.id)}`;
  }
  if (conversation.extra?.companion_id) {
    return `/nomi?companion=${encodeURIComponent(conversation.extra.companion_id)}&session=${encodeURIComponent(conversation.id)}&mode=cohabit`;
  }
  return `/conversation/${conversation.id}`;
}
