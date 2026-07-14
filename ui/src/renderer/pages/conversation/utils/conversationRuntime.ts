import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import type { TChatConversation } from '@/common/config/storage';

export const isConversationProcessing = (conversation?: Pick<TChatConversation, 'runtime' | 'status'> | null) => {
  return conversation?.runtime?.is_processing === true;
};

/** A complete projection is delivered over `message.stream` for realtime
 * rendering, but it does not own a model turn and intentionally has no later
 * `finish` / `turn.completed` event. */
export const isCompleteMessageProjection = (
  message?: Pick<IResponseMessage, 'stream_complete'> | null
): boolean => message?.stream_complete === true;
