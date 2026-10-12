import { ipcBridge } from '@/common';
import type { TProviderWithModel } from '@/common/config/storage';
import type { ConversationId } from '@/common/types/ids';
import type { CollaborationConfig } from './useCollaborationDraft';

/** A single model with automatic policies stays on ordinary chat. Extra models,
 * a saved plan or an explicit policy use the same durable Execution on both entry points. */
export const shouldStartCollaboration = (config: CollaborationConfig | undefined): config is CollaborationConfig =>
  Boolean(config && (config.execution_model_pool.mode === 'range' || config.execution_template_id !== null
    || config.delegation_policy !== 'automatic' || config.decision_policy !== 'automatic'));

export async function startCollaboration(conversationId: ConversationId, goal: string, workspace: string,
  model: TProviderWithModel, config: CollaborationConfig) {
  const shared = {
    goal: goal.trim(),
    ...(workspace.trim() ? { work_dir: workspace.trim() } : {}),
    delegation_policy: config.delegation_policy,
    decision_policy: config.decision_policy,
    lead_conversation_id: conversationId,
    lead_model: { provider_id: model.id, model: model.use_model },
  };
  if (config.execution_template_id !== null) {
    return ipcBridge.agentExecutionTemplate.createExecution.invoke({ execution_template_id: config.execution_template_id, request: shared });
  }
  return ipcBridge.agentExecution.create.invoke({ ...shared, model_pool: config.execution_model_pool });
}
