import type { ConversationCommandQueueItem } from '../useConversationCommandQueue';

type SteerCommand = Pick<ConversationCommandQueueItem, 'input' | 'files'>;

/** Preserve the submitted snapshot on failure; never restore over newer typing. */
export async function steerOrQueue(
  command: SteerCommand,
  steer: (command: SteerCommand) => Promise<void>,
  enqueue: (command: SteerCommand) => unknown
): Promise<boolean> {
  try {
    await steer(command);
    return true;
  } catch {
    enqueue(command);
    return false;
  }
}
