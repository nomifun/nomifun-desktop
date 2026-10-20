import type { TMessage } from '@/common/chat/chatLib';
import { isNativeCompletionReport } from '@/common/chat/toolPresentation';
import { isToolObservationUnconfirmed } from './processSegments';

export const isInternalInstructionToolCall = (message: TMessage): boolean =>
  message.type === 'tool_call' &&
  typeof message.content.call_id === 'string' &&
  message.content.call_id.startsWith('agent-instructions:');

/** Hide only confirmed bookkeeping, never a task outcome or a failed proposal.
 * A completion report may be a candidate or contain blocked work; its payload
 * cannot prove task success and does not need a second copy in the process. */
export const isTaskControlReceipt = (message: TMessage): boolean => {
  if (message.type !== 'tool_call') return false;
  const tool = message.content;
  if (tool.name === 'update_plan' && tool.status === 'completed') return true;
  return isNativeCompletionReport({ name: tool.name, capabilityId: tool.capability_id, actionId: tool.action_id })
    && tool.status === 'completed' && Number.isSafeInteger(tool.result_seq) && tool.result_seq! > 0
    && !tool.runtime_hold && !tool.error && !isToolObservationUnconfirmed(message, tool.call_id);
};
