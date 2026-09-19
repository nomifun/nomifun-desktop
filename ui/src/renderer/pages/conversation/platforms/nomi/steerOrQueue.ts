import type { ConversationCommandQueueItem } from '../useConversationCommandQueue';

type SteerPayload = Pick<ConversationCommandQueueItem, 'input' | 'files'>;
type SteerCommand = Pick<
  ConversationCommandQueueItem,
  'input' | 'files' | 'requires_review'
>;

/** Preserve failures for review. A lost response is not permission for a new turn. */
export async function steerOrQueue(
  command: SteerCommand,
  steer: (command: SteerPayload) => Promise<void>,
  enqueue: (command: SteerCommand) => unknown
): Promise<boolean> {
  try {
    await steer({ input: command.input, files: command.files });
    return true;
  } catch {
    if (enqueue({ ...command, requires_review: true }) === null) {
      // A full/disabled queue did not retain the draft. Never report success.
      throw new Error('The unconfirmed steering draft could not be retained in the queue');
    }
    return false;
  }
}
