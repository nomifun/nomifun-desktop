import { httpRequest } from '@/common/adapter/httpBridge';
import type { ConversationId } from '@/common/types/ids';
import { parseMessageId } from '@/common/types/ids';
import type { ConversationCreationTask, CreationReceipt, SubmitCreationRequest } from './types';

export const creationTasksKey = (id: ConversationId) => `/api/conversations/${encodeURIComponent(id)}/creation-tasks`;

export function validateCreationTasks(value: unknown, conversationId: ConversationId): ConversationCreationTask[] {
  if (!Array.isArray(value)) throw new TypeError('Invalid creation task response');
  return value.map(task => {
    if (!task || typeof task !== 'object' || task.owner?.kind !== 'conversation_turn' || task.owner.conversation_id !== conversationId || typeof task.creation_task_id !== 'string' || !['queued', 'running', 'succeeded', 'failed', 'canceled'].includes(task.status) || !Array.isArray(task.result_asset_ids)) {
      throw new TypeError('Creation task response does not belong to this conversation');
    }
    parseMessageId(task.owner.message_id);
    if (task.status === 'succeeded' && task.result_asset_ids.length === 0) throw new TypeError('Completed creation task has no media');
    return task as ConversationCreationTask;
  });
}

export async function listCreationTasks(id: ConversationId): Promise<ConversationCreationTask[]> {
  const page = await httpRequest<{ items: unknown }>('GET', creationTasksKey(id));
  return validateCreationTasks(page.items, id);
}

export async function submitCreation(id: ConversationId, request: SubmitCreationRequest, idempotencyKey: string): Promise<CreationReceipt> {
  const receipt = await httpRequest<CreationReceipt>('POST', creationTasksKey(id), request, { idempotencyKey });
  parseMessageId(receipt.message_id);
  const tasks = validateCreationTasks(receipt.tasks, id);
  if (!tasks.length || tasks.some(task => task.owner.message_id !== receipt.message_id)) throw new TypeError('Invalid creation admission receipt');
  return { message_id: receipt.message_id, tasks };
}

export async function cancelCreation(id: ConversationId, taskId: string): Promise<ConversationCreationTask> {
  const task = await httpRequest<unknown>('POST', `${creationTasksKey(id)}/${encodeURIComponent(taskId)}/cancel`);
  return validateCreationTasks([task], id)[0];
}
