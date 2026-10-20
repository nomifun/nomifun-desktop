import type { TMessage } from '@/common/chat/chatLib';
import type { MessageId } from '@/common/types/ids';
import type { CreativeStudioAgentMessage } from './types';
import { projectProcessSegments, orderProcessSegments, type ProcessSegmentMessage } from '@/renderer/pages/conversation/Messages/processSegments';
import { isInternalInstructionToolCall, isTaskPlanControlReceipt } from '@/renderer/pages/conversation/Messages/toolMessageVisibility';
import { assignTurnIdsFromUserRequests, type TurnDisclosureInputItem } from '@/renderer/pages/conversation/Messages/turnDisclosureModel';

/** Canvas history proves public prose identity only. Process ownership comes
 * from the same canonical message or the runtime's exact active accepted root. */
export function creativeMessageTurn(message: CreativeStudioAgentMessage, canonical: readonly TMessage[], activeTurnId?: MessageId): MessageId | undefined {
  if (message.role !== 'assistant') return undefined;
  if (message.status === 'running') return activeTurnId;
  if (message.status === 'paused') return message.pause.turnId;
  return canonical.find(source => source.message_id === message.id
    && !(source.type === 'text' && source.position === 'right'))?.turn_id;
}

/** One visible member carries each Turn's process. An exact live member owns
 * the active Turn; otherwise use the first explicitly related public member. */
export function creativeProcessOwnerIds(messages: readonly CreativeStudioAgentMessage[], canonical: readonly TMessage[], activeTurnId?: MessageId): ReadonlySet<string> {
  const owners = new Map<MessageId, CreativeStudioAgentMessage>();
  for (const message of messages) {
    const turn = creativeMessageTurn(message, canonical, activeTurnId);
    if (!turn) continue;
    if (!owners.has(turn) || (message.status === 'running' && turn === activeTurnId)) owners.set(turn, message);
  }
  return new Set([...owners.values()].map(message => message.id));
}

export function creativeAcceptedTurnOwners(canonical: readonly TMessage[], activeTurnId?: MessageId, activeRequestMessageId?: MessageId): TurnDisclosureInputItem[] {
  return assignTurnIdsFromUserRequests(canonical.map(message => ({
    id: message.id, turnId: message.type === 'text' && message.position === 'right' ? message.msg_id ?? message.message_id : message.turn_id,
    role: message.type === 'text' ? message.position === 'right' ? 'user' : 'assistant' : 'metadata',
    createdAt: message.created_at ?? 0,
    sourceMessageIds: [message.message_id, message.msg_id].filter((id): id is MessageId => Boolean(id)),
  })), { activeTurnId, activeRequestMessageId });
}

export function creativeProcessSegments(canonical: TMessage[], visibleReasoning: boolean): ProcessSegmentMessage[] {
  // Public Canvas prompts and planning responses keep their existing product
  // projection. Never render canonical model-input/planning JSON as prose here.
  return orderProcessSegments(projectProcessSegments(canonical, new Set()).filter(message =>
    !message.hidden && !isInternalInstructionToolCall(message) && !isTaskPlanControlReceipt(message)
    && (message.type === 'tool_call' || message.type === 'tool_group' || (visibleReasoning && message.type === 'thinking'))));
}
