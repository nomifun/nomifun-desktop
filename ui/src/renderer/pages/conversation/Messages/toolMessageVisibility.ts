import type { TMessage } from '@/common/chat/chatLib';

export const isInternalInstructionToolCall = (message: TMessage): boolean =>
  message.type === 'tool_call' &&
  typeof message.content.call_id === 'string' &&
  message.content.call_id.startsWith('agent-instructions:');

/** Successful progress declarations live in the task bar. Rejections remain
 * visible as tool errors; no legacy synthetic-error matching is needed. */
export const isTaskPlanControlReceipt = (message: TMessage): boolean =>
  message.type === 'tool_call' && message.content.name === 'update_plan' &&
  message.content.status === 'completed';
