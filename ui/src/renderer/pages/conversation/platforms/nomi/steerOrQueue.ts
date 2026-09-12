import type { ConversationCommandQueueItem } from '../useConversationCommandQueue';

type SteerPayload = Pick<ConversationCommandQueueItem, 'input' | 'files'>;
type SteerCommand = Pick<
  ConversationCommandQueueItem,
  'input' | 'files' | 'capability_selection'
>;

/** Preserve the submitted snapshot on failure; never restore over newer typing. */
export async function steerOrQueue(
  command: SteerCommand,
  steer: (command: SteerPayload) => Promise<void>,
  enqueue: (command: SteerCommand) => unknown
): Promise<boolean> {
  try {
    await steer({ input: command.input, files: command.files });
    return true;
  } catch {
    enqueue(command);
    return false;
  }
}
