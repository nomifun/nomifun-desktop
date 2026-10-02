import { agentPlatform, conversation } from '@/common/adapter/ipcBridge';
import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import type { NativeAgentExecution } from '@/common/types/agentPlatform';

type Message = Parameters<typeof conversation.sendMessage.invoke>[0] & { idempotency_key: string };
const pluginPauses = new Set([
  'PLUGIN_AUTHORIZATION_REQUIRED', 'PLUGIN_VERIFICATION_REQUIRED', 'PLUGIN_DELIVERY_REQUIRED',
  'PLUGIN_CURRENT_CONVERSATION_PENDING',
  'OWNER_REQUESTED',
]);

function resumeRequest(execution: NativeAgentExecution, message: Message) {
  if (!execution.pause || !pluginPauses.has(execution.pause.reason)
      || !execution.pause.cleanup_proven || !execution.checkpoint_retained
      || !execution.checkpoint_digest || execution.checkpoint_revision < 1) {
    throw new Error('PLUGIN_CONTINUATION_NOT_AVAILABLE');
  }
  return {
      operation_id: execution.operation_id,
      idempotency_key: message.idempotency_key,
      expected_pause_revision: execution.pause.revision,
      expected_checkpoint_revision: execution.checkpoint_revision,
      expected_checkpoint_digest: execution.checkpoint_digest,
      budget: {} as Record<string, never>,
  };
}

async function resumePaused(execution: NativeAgentExecution, message: Message) {
  await agentPlatform.sessions.resumeExecution.invoke({
    agent_session_id: message.conversation_id, request: resumeRequest(execution, message),
  });
}

/** The supplied pause is retained for same-key retry after an ambiguous response. */
export async function replyToPluginConversation(message: Message, execution: NativeAgentExecution) {
  if (!execution.pause?.reason.startsWith('PLUGIN_') || !pluginPauses.has(execution.pause.reason)) {
    throw new Error('PLUGIN_CONTINUATION_NOT_AVAILABLE');
  }
  return pluginPlatform.authoring.continueWithInput.invoke({
    agent_session_id: message.conversation_id, request: resumeRequest(execution, message),
    input: { content: message.input, ...(message.files?.length ? { files: message.files } : {}) },
  });
}

/** A paused task retains its turn. Starting or steering a second turn is invalid. */
export async function continuePluginConversation(message: Message) {
  const parameters = { agent_session_id: message.conversation_id };
  const execution = await agentPlatform.sessions.getExecution.invoke(parameters);
  if (execution?.state === 'paused') return resumePaused(execution, message);
  try {
    await conversation.sendMessage.invoke(message);
  } catch (caught) {
    if (!(typeof caught === 'object' && caught !== null && 'status' in caught && caught.status === 409)) throw caught;
    const current = await agentPlatform.sessions.getExecution.invoke(parameters);
    if (current?.state === 'paused') return resumePaused(current, message);
    await conversation.steer.invoke(message);
  }
}
