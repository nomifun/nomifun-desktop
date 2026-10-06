import type { ConversationId, MessageId } from '@/common/types/ids';
export const CHAT_MESSAGE_JUMP_EVENT = 'nomifun-chat-message-jump';

export interface ChatMessageJumpDetail {
  conversation_id: ConversationId;
  messageId?: MessageId;
  msgId?: MessageId;
  align?: 'start' | 'center' | 'end';
  behavior?: 'auto' | 'smooth';
  /** Explicit question references may request older canonical history pages. */
  loadOlder?: boolean;
}

export function dispatchChatMessageJump(detail: ChatMessageJumpDetail) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent<ChatMessageJumpDetail>(CHAT_MESSAGE_JUMP_EVENT, {
      detail,
    })
  );
}
