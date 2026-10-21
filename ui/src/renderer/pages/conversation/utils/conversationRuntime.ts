import type { IResponseMessage } from '@/common/adapter/ipcBridge';
import type { AgentStreamErrorInfo } from '@/common/chat/chatLib';
import type { TChatConversation } from '@/common/config/storage';
import { OFFICIAL_PRESET_KEYS, type OfficialPresetKey } from '@/common/types/agentPlatform';

export type ConversationRuntimeAuthority = 'idle' | 'processing' | 'unknown';

type ActivitySnapshot = Pick<TChatConversation, 'runtime' | 'status'> & Partial<Pick<TChatConversation, 'extra'>>;
export type ConversationActivity = 'idle' | 'running' | 'waiting' | 'paused';

/** Presentation derived from the existing canonical projection, never a second
 * lifecycle. Undefined means the snapshot has not established activity. */
export const getConversationActivity = (conversation?: ActivitySnapshot | null): ConversationActivity | undefined => {
  if (!conversation) return undefined;
  const runtime = conversation.runtime;
  // A retained, exact input-preparation owner can outlive the preceding Turn's
  // finished status. Its read projection blocks idle actions until cleanup.
  if (runtime?.state === 'starting' && runtime.is_processing === true) return 'running';
  if (conversation.status === 'finished') return 'idle';
  if (runtime?.state === 'waiting') return runtime.active_turn_id && runtime.is_processing === false
    && runtime.can_send_message === false && runtime.turn_outcome?.kind === 'waiting' ? 'waiting' : undefined;
  if (runtime?.state === 'paused' || runtime?.state === 'reconciliation') return runtime.active_turn_id && runtime.is_processing === false
    && runtime.can_send_message === false ? 'paused' : undefined;
  if (conversation.status === 'running') {
    if (runtime?.is_processing === true && runtime.active_turn_id) return 'running';
    return undefined;
  }
  if (conversation.status === 'pending' && runtime?.is_processing !== true && runtime?.active_turn_id == null) return 'idle';
  return undefined;
};

/** Paused and unverified ownership blocks configuration as well as execution. */
export const isConversationBindingBlocked = (
  conversation?: ActivitySnapshot & Partial<Pick<TChatConversation, 'execution_attempt_id'>> | null
): boolean => getConversationActivity(conversation) !== 'idle'
  || Boolean(conversation?.runtime?.active_turn_id || conversation?.execution_attempt_id);

export const getConversationPauseNotice = (
  conversation?: Pick<TChatConversation, 'runtime' | 'status' | 'extra' | 'agent_snapshot'> | null
) => {
  const runtime = conversation?.runtime;
  if (getConversationActivity(conversation) !== 'paused' || !conversation || !runtime?.active_turn_id) return null;
  const pause = conversation.extra.execution_pause;
  // This snapshot still belongs to the active paused Turn: the canonical owner
  // blocks Agent/model/resource transitions until that Turn relinquishes it.
  const snapshot = conversation.agent_snapshot;
  const savedAgentLabel = snapshot?.preset_name;
  const savedModelName = snapshot?.resolved_model?.model;
  const agentLabel = typeof savedAgentLabel === 'string' && savedAgentLabel.trim() ? savedAgentLabel : undefined;
  const modelName = typeof savedModelName === 'string' && savedModelName.trim() ? savedModelName : undefined;
  const templateKey = 'official_template_key' in conversation.extra
    ? conversation.extra.official_template_key : undefined;
  const agentTemplateKey = OFFICIAL_PRESET_KEYS.includes(templateKey as OfficialPresetKey)
    ? templateKey as OfficialPresetKey : undefined;
  const workspacePath = typeof conversation.extra.workspace === 'string' && conversation.extra.workspace.length
    ? conversation.extra.workspace : undefined;
  const error: AgentStreamErrorInfo | undefined = agentLabel || modelName || agentTemplateKey || workspacePath
    ? {
        message: '',
        ...(agentLabel ? { agentLabel } : {}),
        ...(modelName ? { modelName } : {}),
        ...(agentTemplateKey ? { agentTemplateKey } : {}),
        ...(workspacePath ? { workspacePath } : {}),
      }
    : undefined;
  return {
    turnId: runtime.active_turn_id,
    reason: runtime.state === 'reconciliation' ? 'NATIVE_RECOVERY_RECONCILIATION_REQUIRED'
      : typeof pause?.reason === 'string' && /^[A-Z][A-Z0-9_]{0,127}$/.test(pause.reason) ? pause.reason : undefined,
    cleanupProven: pause?.cleanup_proven === true,
    pausedAt: typeof pause?.paused_at_ms === 'number' && Number.isFinite(pause.paused_at_ms) && pause.paused_at_ms > 0
      ? pause.paused_at_ms : undefined,
    ...(error ? { error } : {}),
  };
};

export type ConversationPauseNotice = NonNullable<ReturnType<typeof getConversationPauseNotice>>;

export const getConversationWaitingNotice = (conversation?: ActivitySnapshot | null) => {
  const runtime = conversation?.runtime;
  if (getConversationActivity(conversation) !== 'waiting' || !runtime?.active_turn_id || runtime.turn_outcome?.kind !== 'waiting') return null;
  return { turnId: runtime.active_turn_id, outcome: runtime.turn_outcome };
};
export type ConversationWaitingNotice = NonNullable<ReturnType<typeof getConversationWaitingNotice>>;

export const getConversationRuntimeAuthority = (
  conversation?: Pick<TChatConversation, 'runtime' | 'status'> | null
): ConversationRuntimeAuthority => {
  if (!conversation) return 'idle';

  const activity = getConversationActivity(conversation);
  // The queue's existing authority contract deliberately excludes suspension.
  return activity === 'running' || activity === 'waiting' ? 'processing' : activity === 'idle' ? 'idle' : 'unknown';
};

export const isConversationProcessing = (conversation?: Pick<TChatConversation, 'runtime' | 'status'> | null) => {
  return getConversationRuntimeAuthority(conversation) === 'processing';
};

/** A complete projection is delivered over `message.stream` for realtime
 * rendering, but it does not own a model turn and intentionally has no later
 * `finish` / `turn.completed` event. */
export const isCompleteMessageProjection = (
  message?: Pick<IResponseMessage, 'stream_complete'> | null
): boolean => message?.stream_complete === true;
