import type { AgentStreamErrorInfo, IMessageTips } from '@/common/chat/chatLib';
import type { ConversationId } from '@/common/types/ids';
import type { ConversationPauseNotice } from './conversationRuntime';

// Exact canonical broker codes only. Similar prose is never a classifier.
const MODEL_PAUSE_CODES: Readonly<Record<string, string>> = {
  EXECUTION_MODEL_CAUSALITY_REJECTED: 'NOMIFUN_STATE_INCONSISTENT',
  EXECUTION_MODEL_DUPLICATE_OPERATION: 'NOMIFUN_STATE_INCONSISTENT',
  EXECUTION_MODEL_SHADOW_NOT_PRIMARY: 'NOMIFUN_STATE_INCONSISTENT',
  EXECUTION_MODEL_SESSION_TERMINAL: 'NOMIFUN_STATE_INCONSISTENT',
  EXECUTION_MODEL_ROUTE_NOT_FOUND: 'USER_LLM_PROVIDER_CONFIG_ERROR',
  EXECUTION_MODEL_ROUTE_REVISION_MISMATCH: 'NOMIFUN_SESSION_CONFIGURATION_CHANGED',
  EXECUTION_MODEL_ADAPTER_UNAVAILABLE: 'USER_LLM_PROVIDER_CONFIG_ERROR',
  EXECUTION_MODEL_CREDENTIAL_REFERENCE_MISSING: 'USER_LLM_PROVIDER_CONFIG_ERROR',
  EXECUTION_MODEL_CREDENTIAL_TARGET_MISMATCH: 'USER_LLM_PROVIDER_CONFIG_ERROR',
  EXECUTION_MODEL_UNSUPPORTED_FEATURE: 'EXECUTION_MODEL_UNSUPPORTED_FEATURE',
  EXECUTION_MODEL_INVALID_REQUEST: 'USER_LLM_PROVIDER_INVALID_REQUEST',
  EXECUTION_MODEL_AUTHENTICATION_FAILED: 'USER_LLM_PROVIDER_AUTH_FAILED',
  EXECUTION_MODEL_RATE_LIMITED: 'USER_LLM_PROVIDER_RATE_LIMITED',
  EXECUTION_MODEL_PROMPT_TOO_LONG: 'USER_LLM_PROVIDER_CONTEXT_TOO_LARGE',
  EXECUTION_MODEL_PROVIDER_UNAVAILABLE: 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE',
  EXECUTION_MODEL_PROTOCOL_VIOLATION: 'EXECUTION_MODEL_PROTOCOL_VIOLATION',
  EXECUTION_MODEL_STREAM_INTERRUPTED: 'EXECUTION_MODEL_STREAM_INTERRUPTED',
  EXECUTION_MODEL_CANCELLED: 'EXECUTION_PAUSED',
  EXECUTION_MODEL_INTERNAL: 'NOMIFUN_INTERNAL_ERROR',
  EXECUTION_MODEL_STREAM_ENDED_WITHOUT_TERMINAL: 'EXECUTION_MODEL_STREAM_ENDED_WITHOUT_TERMINAL',
  EXECUTION_MODEL_INVALID_EVENT: 'EXECUTION_MODEL_INVALID_EVENT',
};

const PUBLIC_PAUSE_REASONS = new Set([
  ...Object.keys(MODEL_PAUSE_CODES),
  'EXECUTION_PAUSED',
  'EXECUTION_USER_REQUESTED',
  'EXECUTION_ATTACH_FAILED',
  'EXECUTION_PREPARATION_BLOCKED',
  'EXECUTION_SESSION_PAYLOAD_BUDGET',
]);

const publicPauseReason = (reason: string | undefined): string | undefined =>
  reason && PUBLIC_PAUSE_REASONS.has(reason) ? reason : undefined;

/** Display an owner-verified suspension without creating a failed/completed Turn. */
export const conversationPauseError = (pause: ConversationPauseNotice): AgentStreamErrorInfo => {
  const reason = publicPauseReason(pause.reason);
  const code = !pause.cleanupProven
    ? 'EXECUTION_CLEANUP_UNCONFIRMED'
    : reason ? MODEL_PAUSE_CODES[reason] ?? reason : 'EXECUTION_PAUSED';
  return {
    ...pause.error,
    message: '',
    code,
    detail: reason,
    resolution: undefined,
    retryable: false,
    feedback_recommended: false,
  };
};

/** Frontend-local presentation, never a durable Message identity or terminal receipt. */
export const conversationPauseErrorMessage = (
  conversationId: ConversationId,
  pause: ConversationPauseNotice
): IMessageTips => ({
  id: `pause-error:${pause.turnId}:${pause.pausedAt ?? 'unknown'}`,
  type: 'tips',
  conversation_id: conversationId,
  turn_id: pause.turnId,
  created_at: pause.pausedAt,
  position: 'center',
  content: {
    type: 'error',
    content: '',
    error: conversationPauseError(pause),
    execution_pause: {
      reason: publicPauseReason(pause.reason),
      cleanupProven: pause.cleanupProven,
    },
  },
});
