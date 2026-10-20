import type { ConversationId, MessageId } from '../types/ids';
import type { ToolDetailSection } from '../protocolBindings/ToolDetailSection';
export type { ToolDetailSection } from '../protocolBindings/ToolDetailSection';
export type { ToolDetailPage } from '../protocolBindings/ToolDetailPage';

/** Read-only canonical payload pages. They never become transcript content. */
export interface ToolDetailIdentity {
  conversation_id: ConversationId;
  turn_id: MessageId;
  call_id: string;
  through_seq?: number;
  expected_result_seq?: number;
}

export type ToolDetailRequest = ToolDetailIdentity & { section: ToolDetailSection; cursor?: string; limit?: number };

export const toolDetailIdentityKey = (source: ToolDetailIdentity): string => JSON.stringify([
  source.conversation_id, source.turn_id, source.call_id, source.expected_result_seq ?? null,
]);

/** A read boundary is separate from the immutable call/result that the user selected. */
export const toolDetailSnapshotKey = (source: ToolDetailIdentity): string => JSON.stringify([
  toolDetailIdentityKey(source), source.through_seq ?? null,
]);
