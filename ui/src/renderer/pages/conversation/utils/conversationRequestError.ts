import { normalizeAgentStreamError, type AgentStreamErrorInfo, type TMessage } from '@/common/chat/chatLib';
import { isBackendHttpError } from '@/common/adapter/httpBridge';

const record = (value: unknown): Record<string, unknown> | undefined =>
  value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined;

/** Rejected requests are transient presentation, never fabricated Turn history. */
export function conversationRequestError(error: unknown, fallbackCode: string): AgentStreamErrorInfo {
  const payload = isBackendHttpError(error) ? record(error.body) : record(error);
  const details = record(isBackendHttpError(error) ? error.details : payload?.details);
  const typed = normalizeAgentStreamError(payload?.error) ?? normalizeAgentStreamError(details?.error)
    ?? normalizeAgentStreamError(payload);
  const message = isBackendHttpError(error) ? error.backendMessage || error.message
    : error instanceof Error ? error.message : typeof error === 'string' ? error : JSON.stringify(error) || '';
  const code = typeof payload?.code === 'string' && !['CONFLICT', 'INTERNAL_ERROR', 'BAD_GATEWAY'].includes(payload.code)
    ? payload.code : fallbackCode;
  return {
    ...(typed ?? { message, detail: message, code }),
    ...(!typed?.workspacePath && typeof details?.workspace_path === 'string' ? { workspacePath: details.workspace_path } : {}),
    retryable: false,
  };
}

export interface ConversationRequestFailure {
  error: AgentStreamErrorInfo;
  timestamp: number;
  /** Existing identities fence late responses from a previous request. */
  previousMessageIds?: ReadonlySet<string>;
  input?: string;
}

/** An exact newly admitted request's canonical error replaces its HTTP fallback. */
export function hasCanonicalRequestError(failure: ConversationRequestFailure, messages: readonly TMessage[]): boolean {
  if (failure.input === undefined || !failure.previousMessageIds) return false;
  return messages.some(message => {
    if (message.type !== 'tips' || message.content.type !== 'error' || !message.turn_id
      || !message.content.error || message.content.idmm_notice || failure.previousMessageIds?.has(message.turn_id)) return false;
    return messages.some(request => request.type === 'text' && request.position === 'right'
      && (request.message_id ?? request.msg_id) === message.turn_id
      && request.content.content === failure.input);
  });
}
