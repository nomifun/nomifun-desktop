import type { TMessage } from '../chat/chatLib';
import type { TChatConversation } from '../config/storage';
import type { MessageId } from './ids';

export interface IMessageSearchItem {
  conversation: TChatConversation;
  message_id: MessageId;
  message_type: TMessage['type'];
  message_created_at: number;
  preview_text: string;
}
