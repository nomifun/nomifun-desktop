import type { MessageId } from '@/common/types/ids';

export interface CreationTaskPlacement {
  turnId: MessageId;
  messageId: MessageId;
}

/**
 * Place each Conversation-owned creation task after the last visible item in
 * its turn. The task persists the initiating user message id, while the
 * renderer groups process rows and the final assistant reply by the canonical
 * turn id; this mapping keeps those two identities explicit instead of
 * attaching media directly beneath the user bubble.
 */
export function creationTaskPlacementAfterIndices(
  itemTurnIds: readonly (MessageId | undefined)[],
  ownerMessageIdByTurn: ReadonlyMap<MessageId, MessageId>
): ReadonlyMap<number, CreationTaskPlacement> {
  const lastIndexByTurn = new Map<MessageId, number>();
  itemTurnIds.forEach((turnId, index) => {
    if (turnId && ownerMessageIdByTurn.has(turnId)) lastIndexByTurn.set(turnId, index);
  });

  const placements = new Map<number, CreationTaskPlacement>();
  for (const [turnId, index] of lastIndexByTurn) {
    const messageId = ownerMessageIdByTurn.get(turnId);
    if (messageId) placements.set(index, { turnId, messageId });
  }
  return placements;
}
