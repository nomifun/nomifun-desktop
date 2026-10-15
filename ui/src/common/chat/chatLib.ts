/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  PersistedToolArtifact,
} from '@/common/types/platform/toolCallTypes';
import { OFFICIAL_PRESET_KEYS, type OfficialPresetKey } from '@/common/types/agentPlatform';
import { normalizeIdmmDecisionExplanation, normalizeIdmmDecisionNotice, type IdmmDecisionExplanation, type IdmmDecisionNotice } from '@/common/types/idmm';
import type { IResponseMessage, IUserMessageCreatedEvent } from '../adapter/ipcBridge';
import {
  parseConversationId,
  parseCronJobId,
  parseKnowledgeBaseId,
  parseMessageId,
  parsePersistedArtifactId,
  type ConversationId,
  type CronJobId,
  type KnowledgeBaseId,
  type MessageId,
} from '../types/ids';
import { uuid } from '../utils';
import { optionalDisplayText, toDisplayText } from './displayText';
import { normalizeToolGroupStatus } from './toolGroupStatus';
import { normalizeModelFailureDiagnostic, sanitizeDiagnosticDetail, type ModelFailureDiagnostic } from './providerDiagnostic';
import { normalizeKnowledgeWritebackDiagnostic, type KnowledgeWritebackDiagnostic } from './knowledgeWritebackDiagnostic';
import { isAbsoluteLocalPath, isFileUri } from '../utils/localPath';
import { computerPermissionKindsForOperation } from './computerPermissionContract';
import type { AgentUserInputRequest } from '../protocolBindings/AgentUserInputRequest';
import type { AgentTurnOutcome } from '../protocolBindings/AgentTurnOutcome';
import { normalizeAgentTurnOutcome, mergeAgentTurnOutcome } from './turnOutcome';

export { joinLocalPath as joinPath } from '../utils/localPath';

/**
 * @description 跟对话相关的消息类型申明 及相关处理
 */

type TMessageType =
  | 'text'
  | 'tips'
  | 'tool_call'
  | 'tool_group'
  | 'agent_status'
  | 'thinking';

interface IMessage<T extends TMessageType, Content extends Record<string, any>> {
  /**
   * 唯一ID — frontend-local render key, NOT a backend entity id.
   */
  id: string;
  /** Durable message entity identity, present for persisted history rows. */
  message_id?: MessageId;
  /** Stable backend message UUIDv7. */
  msg_id?: MessageId;

  /** Stable backend turn correlation. Message identity and turn identity are
   * intentionally separate: multiple durable rows belong to one turn. */
  turn_id?: MessageId;

  /** Owning canonical Conversation entity id. */
  conversation_id: ConversationId;
  /**
   * 消息类型
   */
  type: T;
  /**
   * 消息内容
   */
  content: Content;
  /**
   * 消息创建时间
   */
  created_at?: number;
  /**
   * 消息位置
   */
  position?: 'left' | 'right' | 'center' | 'pop';
  /**
   * 消息状态
   */
  status?: 'finish' | 'pending' | 'error' | 'work';
  /**
   * Hidden from UI display but persisted to DB and sent to agent.
   */
  hidden?: boolean;
}

export type CronMessageMeta = {
  source: 'cron';
  cron_job_id: CronJobId;
  cron_job_name: string;
  triggered_at: number;
};

type KnowledgeWritebackStatus =
  | 'running'
  | 'written'
  | 'partial'
  | 'failed'
  | 'no_candidate'
  | 'no_completer'
  | 'cancelled'
  | 'interrupted';

type KnowledgeWritebackFile = {
  kb_id: KnowledgeBaseId;
  rel_path: string;
};

export type KnowledgeWritebackState = {
  status: KnowledgeWritebackStatus;
  written: KnowledgeWritebackFile[];
  detail?: string;
  diagnostic?: KnowledgeWritebackDiagnostic;
  /** Canonical Event sequence; absent only on a pre-decoupling terminal receipt. */
  event_seq?: number;
};

export type IMessageText = IMessage<
  'text',
  {
    content: string;
    /** Actual message time; created_at remains the durable pagination cursor. */
    display_at_ms?: number;
    /** Read-only paths derived by the authenticated canonical Session projection. */
    managed_file_paths?: Record<string, string>;
    idmm_decision?: IdmmDecisionExplanation;
    /** Canonical typed output-limit continuation, never inferred from text. */
    continuation_of_message_id?: MessageId;
    /** Explicit complete projection without a Runtime Turn (for example AgentExecution synthesis). */
    standalone_projection?: true;
    interaction?: {
      kind: 'robot' | 'desktop'; robot_id: string; connection_id: string; request_id: string;
      input_modality: 'speech' | 'text'; output_mode: 'spoken' | 'desktop';
    };
    observations?: Array<{
      question: string; answer: string; observed_at: number;
      image: { id: string; path: string; mime_type: string; sha256: string };
    }>;
    /** Backend explicitly replaced the accumulated text for this msg_id. */
    replace?: boolean;
    cronMeta?: CronMessageMeta;
    /** True when this reply was sent by another Agent participating in the task. */
    agentMessage?: boolean;
    senderName?: string;
    senderAgentType?: string;
    /** Sender Agent's conversation id — lets the renderer resolve preset avatars via conversation extras. */
    senderConversationId?: ConversationId;
  }
>;

type AgentErrorOwnership = 'nomifun' | 'user_agent' | 'user_llm_provider' | 'unknown_upstream';

type AgentErrorResolutionKind =
  | 'retry'
  | 'wait_for_current_response'
  | 'start_new_session'
  | 'reconnect_agent'
  | 'check_agent_login'
  | 'check_agent_installation'
  | 'check_agent_version'
  | 'check_local_command'
  | 'check_provider_credentials'
  | 'check_provider_billing'
  | 'check_provider_base_url'
  | 'change_model'
  | 'reduce_context'
  | 'send_feedback';

type AgentErrorResolutionTarget = 'provider_settings' | 'agent_settings' | 'new_conversation' | 'feedback';

type AgentErrorResolution = {
  kind: AgentErrorResolutionKind;
  target?: AgentErrorResolutionTarget;
};

export const AGENT_TASK_INCOMPLETE_REASONS = [
  'blocked_work', 'step_limit', 'output_truncated', 'unresolved_plan',
  'unverified_changes', 'running_processes', 'unverified_completion',
  'rejected_control', 'no_progress', 'execution_guard', 'recovery_guard',
] as const;
export type AgentTaskIncompleteReason = (typeof AGENT_TASK_INCOMPLETE_REASONS)[number];

export type AgentStreamErrorInfo = {
  message: string;
  code?: string;
  ownership?: AgentErrorOwnership;
  detail?: string;
  workspacePath?: string;
  taskIncompleteReason?: AgentTaskIncompleteReason;
  providerDiagnostic?: ModelFailureDiagnostic;
  /** Frozen execution identity supplied by the host, never the current selection. */
  agentLabel?: string;
  agentTemplateKey?: OfficialPresetKey;
  modelName?: string;
  retryable?: boolean;
  feedback_recommended?: boolean;
  resolution?: AgentErrorResolution;
};

type TruncatedTurnFailureCode = 'output_truncated' | 'turn_requests_exhausted';

export type TruncatedTurnRecovery = {
  kind: 'continue_truncated';
  source_message_id: MessageId;
  failure_code: TruncatedTurnFailureCode;
};

export type IMessageTips = IMessage<
  'tips',
  {
    content: string;
    type: 'error' | 'success' | 'warning';
    error?: AgentStreamErrorInfo;
    /** Read/presentation failure; this notice does not rewrite the Turn terminal. */
    query_error?: true;
    /** Presentation of a verified active pause; never a terminal receipt. */
    execution_pause?: { reason?: string; cleanupProven: boolean };
    recovery?: TruncatedTurnRecovery;
    idmm_notice?: IdmmDecisionNotice;
    /** Host-authored input handoff from the exact completed Turn. */
    user_input_request?: AgentUserInputRequest;
    turn_outcome?: AgentTurnOutcome;
    agent_transition?: {
      transition_id: string;
      previous_agent_label: string;
      next_agent_label: string;
      previous_preset_id?: string;
      next_preset_id?: string;
      previous_template_key?: OfficialPresetKey;
      next_template_key?: OfficialPresetKey;
      effective_from: 'next_turn';
      handoff_mode: 'continue_task' | 'context_only';
      completion_gate_inherited: false;
    };
    /** Canonical wall-clock interval for the owning turn. */
    started_at_ms?: number;
    finished_at_ms?: number;
  }
>;

/** An authenticated host refusal before the requested Computer action ran. */
export type SystemPermissionRequired = {
  schema: 'system-permission-required.v1';
  capability: 'computer';
  action_id: string;
  operation: string;
  permissions: Array<'screen_recording' | 'accessibility'>;
  execution: 'not_executed';
};

export type IMessageToolCall = IMessage<
  'tool_call',
  {
    call_id: string;
    name: string;
    /** Read-only paths derived by the authenticated canonical Session projection. */
    managed_file_paths?: Record<string, string>;
    /**
     * Provider arguments when they were decoded as a JSON object. Local
     * Pre-execution validation failures preserve the rejected object so users
     * can inspect exactly what the model sent.
     */
    args?: Record<string, unknown> | null;
    /** Canonical identity projected by the host, separate from the model alias. */
    capability_id?: string;
    action_id?: string;
    error?: string;
    error_code?: string;
    permission_required?: SystemPermissionRequired;
    /** Exact canonical tool result event order, shared by live and history. */
    result_seq?: number;
    status?: 'running' | 'completed' | 'error' | 'canceled';
    input?: Record<string, unknown>;
    output?: string;
    description?: string;
    /**
     * Engine-authored retry identity. The UI must require a complete,
     * contiguous chain and never infer retries from timing or similar args.
     */
    retry?: {
      retry_group_id: string;
      attempt_no: number;
      retry_of_call_id?: string;
    };
    /** Verified, durable outputs emitted by the backend. */
    artifacts?: PersistedToolArtifact[];
    /** Persisted rows expose receipts only after the enclosing turn commits. */
    artifact_delivery_committed?: boolean;
  }
>;

/**
 * Merge one generic tool lifecycle without allowing stale success to win.
 * Error is absorbing, but an Error correction is allowed to replace an
 * earlier Completed frame when the enclosing turn later fails.
 */
export const mergeToolCallContent = (
  existing: IMessageToolCall['content'],
  incoming: IMessageToolCall['content']
): IMessageToolCall['content'] => {
  const merged = { ...existing, ...incoming };

  if (existing.status === 'error' || incoming.status === 'error') {
    return { ...merged, status: 'error', artifacts: [] };
  }
  if (existing.status === 'canceled' || incoming.status === 'canceled') {
    return { ...merged, status: 'canceled', artifacts: [] };
  }
  if (existing.status === 'completed' && incoming.status !== 'completed') {
    return {
      ...merged,
      status: 'completed',
      artifacts: existing.artifacts ?? [],
    };
  }
  if (merged.status !== 'completed') {
    merged.artifacts = [];
  }
  return merged;
};

export type IMessageToolGroup = IMessage<
  'tool_group',
  Array<{
    call_id: string;
    description: string;
    name: string;
    status: 'Executing' | 'Success' | 'Error' | 'Canceled' | 'Pending';
    action_id?: string;
    args?: Record<string, unknown> | null;
    input?: Record<string, unknown>;
    error_code?: string;
    permission_required?: SystemPermissionRequired;
    result_seq?: number;
  }>
>;

// Unified agent status message type for the native agent runtime.
export type IMessageAgentStatus = IMessage<
  'agent_status',
  {
    backend: string; // Agent identifier, e.g. 'nomi'
    status:
      | 'connecting'
      | 'connected'
      | 'authenticated'
      | 'session_active'
      | 'preparing'
      | 'prepared'
      | 'disconnected'
      | 'error';
    /** Display name for the agent / Agent 显示名称 */
    agent_name?: string;
    // Optional runtime metadata supplied by the agent.
    session_id?: string;
    is_connected?: boolean;
    has_active_session?: boolean;
    /** Durable, renderer-facing lifecycle receipt derived from the canonical Turn. */
    turn_summary?: boolean;
    turn_outcome?: AgentTurnOutcome;
    /** Display-only write-back progress/results belonging to this canonical Turn. */
    knowledge_writeback?: KnowledgeWritebackState;
    turn_state?: 'running' | 'completed' | 'failed' | 'cancelled' | 'interrupted';
    started_seq?: number;
    finished_seq?: number | null;
    /** Canonical wall-clock interval for the owning turn. */
    started_at_ms?: number;
    finished_at_ms?: number;
  }
>;

type ResponseTextData = {
  content: unknown;
  continuation_of_message_id?: unknown;
  replace?: boolean;
  cronMeta?: CronMessageMeta;
  idmm_decision?: unknown;
  teammate_message?: unknown;
  sender_name?: unknown;
  sender_backend?: unknown;
  /**
   * Untrusted wire field. It is validated into `ConversationId` by
   * `normalizeWireAgentMessageMetadata` before entering renderer state.
   */
  sender_conversation_id?: string;
};

type AgentMessageMetadata = Pick<
  IMessageText['content'],
  'agentMessage' | 'senderName' | 'senderAgentType' | 'senderConversationId'
>;

const normalizeCronMessageMeta = (value: unknown): CronMessageMeta | undefined => {
  if (!isObject(value) || value.source !== 'cron') return undefined;
  if (typeof value.cron_job_name !== 'string' || typeof value.triggered_at !== 'number') return undefined;
  return {
    source: value.source,
    cron_job_id: parseCronJobId(value.cron_job_id),
    cron_job_name: value.cron_job_name,
    triggered_at: value.triggered_at,
  };
};

/**
 * Translate the external collaboration wire fields into the renderer's
 * single Agent message shape. The wire field names must not propagate
 * beyond message-ingress adapters.
 */
export const normalizeWireAgentMessageMetadata = (
  data: Record<string, unknown>
): Partial<AgentMessageMetadata> => {
  let senderConversationId: ConversationId | undefined;
  if (typeof data.sender_conversation_id === 'string') {
    try {
      senderConversationId = parseConversationId(data.sender_conversation_id);
    } catch {
      // Malformed external metadata must not poison an otherwise valid message.
      senderConversationId = undefined;
    }
  }
  return {
    ...(data.teammate_message ? { agentMessage: true } : {}),
    ...(typeof data.sender_name === 'string' ? { senderName: data.sender_name } : {}),
    ...(typeof data.sender_backend === 'string' ? { senderAgentType: data.sender_backend } : {}),
    ...(senderConversationId ? { senderConversationId } : {}),
  };
};

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);

const KNOWLEDGE_WRITEBACK_STATUSES = new Set<KnowledgeWritebackStatus>([
  'running',
  'written',
  'partial',
  'failed',
  'no_candidate',
  'no_completer',
  'cancelled',
  'interrupted',
]);

export const normalizeKnowledgeWritebackState = (value: unknown): KnowledgeWritebackState | undefined => {
  if (!isObject(value) || typeof value.status !== 'string') return undefined;
  if (!KNOWLEDGE_WRITEBACK_STATUSES.has(value.status as KnowledgeWritebackStatus)) return undefined;
  const eventSeq = Number.isSafeInteger(value.event_seq) && (value.event_seq as number) > 0
    ? value.event_seq as number : undefined;
  if (eventSeq === undefined && value.status !== 'written' && value.status !== 'partial') return undefined;
  if (value.event_seq !== undefined && eventSeq === undefined) return undefined;
  const written = Array.isArray(value.written) ? value.written.flatMap(file => {
    if (!isObject(file) || typeof file.kb_id !== 'string' || typeof file.rel_path !== 'string') return [];
    try { return [{ kb_id: parseKnowledgeBaseId(file.kb_id), rel_path: file.rel_path }]; } catch { return []; }
  }) : [];
  const diagnostic = normalizeKnowledgeWritebackDiagnostic(value.diagnostic);
  const detail = diagnostic ? undefined : sanitizeDiagnosticDetail(value.detail);
  return {
    status: value.status as KnowledgeWritebackStatus,
    written,
    ...(diagnostic ? { diagnostic } : {}),
    ...(detail ? { detail } : {}),
    ...(eventSeq !== undefined ? { event_seq: eventSeq } : {}),
  };
};

const preferKnowledgeWritebackState = (
  existing: KnowledgeWritebackState | undefined,
  incoming: KnowledgeWritebackState | undefined
): KnowledgeWritebackState | undefined => {
  if (!existing) return incoming;
  if (!incoming) return existing;

  // Presentation follows the immutable Event chain, never arrival time or a
  // renderer-owned attempt ledger. Old terminal metadata has no event_seq.
  if (existing.event_seq === undefined && incoming.event_seq === undefined) return incoming;
  return (incoming.event_seq ?? 0) > (existing.event_seq ?? 0) ? incoming : existing;
};

export const mergeAgentStatusContent = (
  existing: IMessageAgentStatus['content'],
  incoming: IMessageAgentStatus['content'],
): IMessageAgentStatus['content'] => {
  const knowledgeWriteback = preferKnowledgeWritebackState(existing.knowledge_writeback, incoming.knowledge_writeback);
  const stale = retainsCommittedTurnOutcome(existing.turn_outcome, incoming.turn_outcome);
  const outcome = mergeAgentTurnOutcome(existing.turn_outcome, incoming.turn_outcome);
  const receipt = stale ? existing : incoming;
  return { ...receipt, ...(outcome ? { turn_outcome: outcome } : {}),
    ...(knowledgeWriteback ? { knowledge_writeback: knowledgeWriteback } : {}) };
};

/** A transport error frame can follow the committed public terminal receipt. */
export const mergeTipsContent = (existing: IMessageTips['content'], incoming: IMessageTips['content']): IMessageTips['content'] =>
  retainsCommittedTurnOutcome(existing.turn_outcome, incoming.turn_outcome) ? existing : incoming;

const retainsCommittedTurnOutcome = (existing: AgentTurnOutcome | undefined, incoming: AgentTurnOutcome | undefined): boolean =>
  existing !== undefined && incoming !== existing && mergeAgentTurnOutcome(existing, incoming) === existing;

/** Post-Turn write-back updates carry presentation facts, never activity authority. */
export const isKnowledgeWritebackPresentation = (message: IResponseMessage): boolean => {
  if (message.type !== 'agent_status' || !message.turn_id || !isObject(message.data)) return false;
  return message.data.turn_summary === true && message.data.turn_state === 'completed'
    && normalizeKnowledgeWritebackState(message.data.knowledge_writeback)?.event_seq !== undefined;
};

/** Validate structured handoff metadata at the stream/history JSON boundary. */
export const normalizeAgentUserInputRequest = (value: unknown): AgentUserInputRequest | undefined => {
  if (!isObject(value) || Object.keys(value).some(key => !['request_id', 'question', 'remaining_work'].includes(key))) return undefined;
  const validText = (text: unknown, maxBytes: number, controls: RegExp): text is string =>
    typeof text === 'string' && text.trim().length > 0
      && new TextEncoder().encode(text).length <= maxBytes && !controls.test(text);
  if (!validText(value.request_id, 256, /[\u0000-\u001f\u007f-\u009f]/u)
    || !validText(value.question, 16384, /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u)
    || !validText(value.remaining_work, 16384, /[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u)) return undefined;
  return { request_id: value.request_id, question: value.question, remaining_work: value.remaining_work };
};

/** Plain reply presentation for consumers without a structured note renderer. */
export const agentUserInputRequestText = (value: unknown): string | undefined => {
  const request = normalizeAgentUserInputRequest(value);
  return request ? `${request.question}\n\n${request.remaining_work}` : undefined;
};

/** A handoff note presents a committed result; it never owns live activity. */
export const isUserInputRequestPresentation = (message: IResponseMessage): message is IResponseMessage & {
  type: 'tips'; data: Record<string, unknown> & { user_input_request: AgentUserInputRequest };
} =>
  message.type === 'tips' && !!message.turn_id && !!message.msg_id && isObject(message.data)
  && message.data.type !== 'error' && normalizeAgentUserInputRequest(message.data.user_input_request) !== undefined;

/** The same host-derived result receipt is delivered live and by history. */
export const isTurnOutcomePresentation = (message: IResponseMessage): boolean =>
  !!message.turn_id && !!message.msg_id && (message.type === 'tips' || message.type === 'agent_status')
  && isObject(message.data) && normalizeAgentTurnOutcome(message.data.turn_outcome) !== undefined;

export const isTurnReadErrorPresentation = (message: IResponseMessage): boolean =>
  !!message.turn_id && !!message.msg_id && (message.type === 'tips' || message.type === 'error')
  && isObject(message.data) && message.data.query_error === true;

const isResponseTextData = (data: unknown): data is ResponseTextData =>
  typeof data === 'object' &&
  data !== null &&
  'content' in data &&
  !Array.isArray(data);

const isTextContentReplacement = (content: IMessageText['content'] | undefined): boolean =>
  content?.replace === true;

export const mergeTextMessageContent = (
  existing: IMessageText['content'],
  incoming: IMessageText['content']
): IMessageText['content'] => {
  const { replace: _existingReplace, ...existingRest } = existing;
  const { replace: incomingReplace, ...incomingRest } = incoming;

  return {
    ...existingRest,
    ...incomingRest,
    content: incomingReplace ? incoming.content : existing.content + incoming.content,
    ...(incomingReplace ? { replace: true } : {}),
  };
};

export const preferTextMessageVersion = (primary: IMessageText, secondary: IMessageText): IMessageText => {
  const primaryIsReplace = isTextContentReplacement(primary.content);
  const secondaryIsReplace = isTextContentReplacement(secondary.content);
  const mergePreferredMetadata = (preferred: IMessageText, fallback: IMessageText): IMessageText => {
    const interaction = preferred.content.interaction ?? fallback.content.interaction;
    // Durable history is passed as primary and owns the decision explanation,
    // even while a longer live text fragment wins the body.
    const decision = primary.content.idmm_decision ?? secondary.content.idmm_decision;
    const observations = (preferred.content.observations?.length ?? 0) >= (fallback.content.observations?.length ?? 0)
      ? preferred.content.observations : fallback.content.observations;
    const managedFilePaths = primary.content.managed_file_paths ?? secondary.content.managed_file_paths;
    if (!interaction && !observations && !decision && !managedFilePaths) return preferred;
    return {
      ...preferred,
      content: {
        ...preferred.content,
        ...(interaction ? { interaction } : {}),
        ...(decision ? { idmm_decision: decision } : {}),
        ...(observations ? { observations } : {}),
        ...(managedFilePaths ? { managed_file_paths: managedFilePaths } : {}),
      },
    };
  };

  if (primaryIsReplace !== secondaryIsReplace) {
    return primaryIsReplace ? mergePreferredMetadata(primary, secondary) : mergePreferredMetadata(secondary, primary);
  }

  return secondary.content.content.length > primary.content.content.length
    ? mergePreferredMetadata(secondary, primary)
    : mergePreferredMetadata(primary, secondary);
};

export type IMessageThinking = IMessage<
  'thinking',
  {
    content: string;
    subject?: string;
    duration?: number;
    status: 'thinking' | 'done';
  }
>;

// eslint-disable-next-line max-len
export type TMessage =
  | IMessageText
  | IMessageTips
  | IMessageToolCall
  | IMessageToolGroup
  | IMessageAgentStatus
  | IMessageThinking;

// 统一所有需要用户交互的用户类型
const AGENT_ERROR_OWNERSHIPS = new Set<AgentErrorOwnership>([
  'nomifun',
  'user_agent',
  'user_llm_provider',
  'unknown_upstream',
]);

const AGENT_ERROR_RESOLUTION_KINDS = new Set<AgentErrorResolutionKind>([
  'retry',
  'wait_for_current_response',
  'start_new_session',
  'reconnect_agent',
  'check_agent_login',
  'check_agent_installation',
  'check_agent_version',
  'check_local_command',
  'check_provider_credentials',
  'check_provider_billing',
  'check_provider_base_url',
  'change_model',
  'reduce_context',
  'send_feedback',
]);

const AGENT_ERROR_RESOLUTION_TARGETS = new Set<AgentErrorResolutionTarget>([
  'provider_settings',
  'agent_settings',
  'new_conversation',
  'feedback',
]);

const normalizeAgentErrorResolution = (value: unknown): AgentErrorResolution | undefined => {
  if (!isObject(value) || typeof value.kind !== 'string') {
    return undefined;
  }

  if (!AGENT_ERROR_RESOLUTION_KINDS.has(value.kind as AgentErrorResolutionKind)) {
    return undefined;
  }

  const target =
    typeof value.target === 'string' && AGENT_ERROR_RESOLUTION_TARGETS.has(value.target as AgentErrorResolutionTarget)
      ? (value.target as AgentErrorResolutionTarget)
      : undefined;

  return {
    kind: value.kind as AgentErrorResolutionKind,
    ...(target ? { target } : {}),
  };
};

export const normalizeAgentStreamError = (value: unknown): AgentStreamErrorInfo | undefined => {
  if (!isObject(value) || typeof value.message !== 'string') {
    return undefined;
  }

  const code = typeof value.code === 'string' ? value.code : undefined;
  const ownership =
    typeof value.ownership === 'string' && AGENT_ERROR_OWNERSHIPS.has(value.ownership as AgentErrorOwnership)
      ? (value.ownership as AgentErrorOwnership)
      : undefined;
  const detail = typeof value.detail === 'string' ? value.detail : undefined;
  const workspacePath = typeof value.workspacePath === 'string' ? value.workspacePath : undefined;
  const taskIncompleteReason = code === 'NOMIFUN_TASK_INCOMPLETE'
    && AGENT_TASK_INCOMPLETE_REASONS.includes(value.taskIncompleteReason as AgentTaskIncompleteReason)
    ? value.taskIncompleteReason as AgentTaskIncompleteReason : undefined;
  const agentLabel = typeof value.agentLabel === 'string' ? value.agentLabel : undefined;
  const agentTemplateKey = OFFICIAL_PRESET_KEYS.includes(value.agentTemplateKey as OfficialPresetKey)
    ? value.agentTemplateKey as OfficialPresetKey : undefined;
  const modelName = typeof value.modelName === 'string' ? value.modelName : undefined;
  const retryable = typeof value.retryable === 'boolean' ? value.retryable : undefined;
  const feedback_recommended = typeof value.feedback_recommended === 'boolean' ? value.feedback_recommended : undefined;
  const resolution = normalizeAgentErrorResolution(value.resolution);
  const providerDiagnostic = normalizeModelFailureDiagnostic(value.providerDiagnostic);

  if (
    !code &&
    !ownership &&
    !detail &&
    !workspacePath &&
    !taskIncompleteReason &&
    !providerDiagnostic &&
    !agentLabel &&
    !agentTemplateKey &&
    !modelName &&
    retryable === undefined &&
    feedback_recommended === undefined &&
    !resolution
  ) {
    return undefined;
  }

  return {
    message: value.message,
    ...(code ? { code } : {}),
    ...(ownership ? { ownership } : {}),
    ...(detail ? { detail } : {}),
    ...(workspacePath ? { workspacePath } : {}),
    ...(taskIncompleteReason ? { taskIncompleteReason } : {}),
    ...(providerDiagnostic ? { providerDiagnostic } : {}),
    ...(agentLabel ? { agentLabel } : {}),
    ...(agentTemplateKey ? { agentTemplateKey } : {}),
    ...(modelName ? { modelName } : {}),
    ...(retryable !== undefined ? { retryable } : {}),
    ...(feedback_recommended !== undefined ? { feedback_recommended } : {}),
    ...(resolution ? { resolution } : {}),
  };
};

export const normalizeTruncatedTurnRecovery = (value: unknown): TruncatedTurnRecovery | undefined => {
  if (!isObject(value) || value.kind !== 'continue_truncated') return undefined;
  if (value.failure_code !== 'output_truncated' && value.failure_code !== 'turn_requests_exhausted') {
    return undefined;
  }
  try {
    return {
      kind: 'continue_truncated',
      source_message_id: parseMessageId(value.source_message_id),
      failure_code: value.failure_code,
    };
  } catch {
    return undefined;
  }
};

const normalizeTipType = (value: unknown): IMessageTips['content']['type'] =>
  value === 'success' || value === 'warning' || value === 'error' ? value : 'warning';

const normalizeThinkingStatus = (value: unknown): IMessageThinking['content']['status'] =>
  value === 'done' ? 'done' : 'thinking';

const finiteNumber = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) ? value : undefined;

export const normalizeSystemPermissionRequired = (value: unknown, actionId: unknown): SystemPermissionRequired | undefined => {
  if (!isObject(value) || value.schema !== 'system-permission-required.v1' || value.capability !== 'computer' ||
    typeof value.action_id !== 'string' || !value.action_id.startsWith('computer/') ||
    typeof value.operation !== 'string' || !value.operation.trim() || value.execution !== 'not_executed' ||
    !Array.isArray(value.permissions) || value.permissions.length === 0 ||
    value.permissions.some(kind => kind !== 'screen_recording' && kind !== 'accessibility')) return undefined;
  const required = computerPermissionKindsForOperation(value.action_id, value.operation);
  const permissions = value.permissions;
  if (actionId !== value.action_id || required.length === 0 || permissions.length !== required.length ||
    !required.every(kind => permissions.includes(kind))) return undefined;
  return { schema: value.schema, capability: value.capability, action_id: value.action_id,
    operation: value.operation, permissions: [...new Set(value.permissions)].sort(), execution: value.execution };
};

export const normalizeToolGroupContent = (value: unknown): IMessageToolGroup['content'] => {
  if (!Array.isArray(value)) return [];

  return value
    .filter(isObject)
    .filter((item) => typeof item.call_id === 'string' && item.call_id.trim().length > 0)
    .map((item) => {
      const status = normalizeToolGroupStatus(item.status);
      const description = toDisplayText(item.description);
      return {
        call_id: toDisplayText(item.call_id),
        description,
        name: toDisplayText(item.name, 'Tool'),
        status,
        ...(typeof item.action_id === 'string' ? { action_id: item.action_id } : {}),
        ...(typeof item.result_seq === 'number' && Number.isSafeInteger(item.result_seq) && item.result_seq >= 0 ? { result_seq: item.result_seq } : {}),
        ...(isObject(item.args) ? { args: item.args } : {}),
        ...(isObject(item.input) ? { input: item.input } : {}),
        ...(typeof item.error_code === 'string' ? { error_code: item.error_code } : {}),
        ...(item.error_code === 'SYSTEM_PERMISSION_REQUIRED' && status === 'Error'
          ? { permission_required: normalizeSystemPermissionRequired(item.permission_required, item.action_id) } : {}),
      };
    });
};

const TOOL_ARTIFACT_KINDS = new Set<PersistedToolArtifact['kind']>([
  'image',
  'audio',
  'video',
  'text',
  'file',
]);
const SHA256_RE = /^[a-f\d]{64}$/i;
const URI_SCHEME_RE = /^[A-Za-z][A-Za-z\d+.-]*:/;

/** Reject malformed or non-canonical receipt metadata before it reaches UI. */
const normalizePersistedToolArtifact = (value: unknown): PersistedToolArtifact | undefined => {
  if (!isObject(value)) return undefined;
  let id;
  try {
    id = parsePersistedArtifactId(value.id);
  } catch {
    return undefined;
  }
  const kind = optionalDisplayText(value.kind) as PersistedToolArtifact['kind'] | undefined;
  const mimeType = optionalDisplayText(value.mime_type);
  const artifactPath = optionalDisplayText(value.path);
  const relativePath = optionalDisplayText(value.relative_path);
  const sizeBytes = value.size_bytes;
  const sha256 = optionalDisplayText(value.sha256);

  if (
    !id ||
    !kind ||
    !TOOL_ARTIFACT_KINDS.has(kind) ||
    !mimeType ||
    !mimeType.includes('/') ||
    !artifactPath ||
    !isAbsoluteLocalPath(artifactPath) ||
    !relativePath ||
    isAbsoluteLocalPath(relativePath) ||
    isFileUri(relativePath) ||
    URI_SCHEME_RE.test(relativePath) ||
    relativePath.split(/[\\/]+/).some((part) => part === '..') ||
    typeof sizeBytes !== 'number' ||
    !Number.isSafeInteger(sizeBytes) ||
    sizeBytes <= 0 ||
    !sha256 ||
    !SHA256_RE.test(sha256)
  ) {
    return undefined;
  }

  return {
    id,
    kind,
    mime_type: mimeType,
    path: artifactPath,
    relative_path: relativePath,
    size_bytes: sizeBytes,
    sha256: sha256.toLowerCase(),
  };
};

export const normalizeToolCallContent = (
  value: unknown,
  persistedStatus?: unknown
): IMessageToolCall['content'] => {
  const data = isObject(value) ? value : {};
  const rawStatus =
    data.status === 'running' || data.status === 'completed' || data.status === 'error' || data.status === 'canceled'
      ? data.status
      : undefined;
  let status = rawStatus;
  if (persistedStatus === 'error') {
    status = 'error';
  } else if (persistedStatus !== undefined && persistedStatus !== 'finish' && status === 'completed') {
    status = 'running';
  }

  const terminalSuccess = status === 'completed' && (persistedStatus === undefined || persistedStatus === 'finish');
  const hasArtifactClaim = Object.prototype.hasOwnProperty.call(data, 'artifacts');
  const rawArtifacts = Array.isArray(data.artifacts) ? data.artifacts : [];
  const deliveryCommitted = persistedStatus === undefined || data.artifact_delivery_committed === true;
  const committedTerminalSuccess = terminalSuccess && deliveryCommitted;
  const artifacts = committedTerminalSuccess
    ? rawArtifacts
        .map(normalizePersistedToolArtifact)
        .filter((artifact): artifact is PersistedToolArtifact => artifact !== undefined)
    : [];
  const invalidTerminalClaim =
    terminalSuccess &&
    hasArtifactClaim &&
    (!Array.isArray(data.artifacts) ||
      (rawArtifacts.length > 0 && !deliveryCommitted) ||
      artifacts.length !== rawArtifacts.length);
  if (invalidTerminalClaim) {
    status = 'error';
  }

  return {
    ...data,
    ...(status ? { status } : {}),
    permission_required: data.error_code === 'SYSTEM_PERMISSION_REQUIRED' && status === 'error'
      ? normalizeSystemPermissionRequired(data.permission_required, data.action_id) : undefined,
    result_seq: typeof data.result_seq === 'number' && Number.isSafeInteger(data.result_seq) && data.result_seq >= 0 ? data.result_seq : undefined,
    artifacts: invalidTerminalClaim ? [] : artifacts,
  } as IMessageToolCall['content'];
};

export const normalizeAgentStatusContent = (value: unknown): IMessageAgentStatus['content'] => {
  const data = isObject(value) ? value : {};
  const outcome = normalizeAgentTurnOutcome(data.turn_outcome);
  const knowledgeWriteback = data.turn_summary === true ? normalizeKnowledgeWritebackState(data.knowledge_writeback) : undefined;
  const status =
    data.status === 'connecting' ||
    data.status === 'connected' ||
    data.status === 'authenticated' ||
    data.status === 'session_active' ||
    data.status === 'preparing' ||
    data.status === 'prepared' ||
    data.status === 'disconnected' ||
    data.status === 'error'
      ? data.status
      : 'error';

  return {
    backend: toDisplayText(data.backend, 'agent'),
    status,
    ...(data.agent_name != null ? { agent_name: toDisplayText(data.agent_name) } : {}),
    ...(data.session_id != null ? { session_id: toDisplayText(data.session_id) } : {}),
    ...(typeof data.is_connected === 'boolean' ? { is_connected: data.is_connected } : {}),
    ...(typeof data.has_active_session === 'boolean' ? { has_active_session: data.has_active_session } : {}),
    ...(data.turn_summary === true ? { turn_summary: true } : {}),
    ...(outcome ? { turn_outcome: outcome } : {}),
    ...(knowledgeWriteback ? { knowledge_writeback: knowledgeWriteback } : {}),
    ...(data.turn_summary === true &&
      ['running', 'completed', 'failed', 'cancelled', 'interrupted'].includes(data.turn_state as string)
      ? { turn_state: data.turn_state as IMessageAgentStatus['content']['turn_state'] } : {}),
    ...(finiteNumber(data.started_seq) != null ? { started_seq: finiteNumber(data.started_seq) } : {}),
    ...(data.finished_seq === null
      ? { finished_seq: null }
      : finiteNumber(data.finished_seq) != null
        ? { finished_seq: finiteNumber(data.finished_seq) }
        : {}),
    ...(finiteNumber(data.started_at_ms) != null
      ? { started_at_ms: finiteNumber(data.started_at_ms) }
      : {}),
    ...(finiteNumber(data.finished_at_ms) != null
      ? { finished_at_ms: finiteNumber(data.finished_at_ms) }
      : {}),
  };
};

/**
 * @description 将后端返回的消息转换为前端消息
 * */
export const transformMessage = (message: IResponseMessage): TMessage | undefined => {
  const created_at = message.created_at ?? Date.now();
  const turnIdentity = message.turn_id ? { turn_id: message.turn_id } : {};
  switch (message.type) {
    case 'error': {
      const errorData = message.data;
      const structuredError = normalizeAgentStreamError(errorData);
      const recovery = isObject(errorData) ? normalizeTruncatedTurnRecovery(errorData.recovery) : undefined;
      const notice = isObject(errorData) ? normalizeIdmmDecisionNotice(errorData.idmm_notice) : undefined;
      const errorText =
        (isObject(errorData) ? optionalDisplayText(errorData.message) : undefined) ?? toDisplayText(errorData);
      return {
        id: uuid(),
        type: 'tips',
        msg_id: message.msg_id,
        ...turnIdentity,
        position: 'center',
        conversation_id: message.conversation_id,
        created_at,
        content: {
          content: errorText,
          type: 'error',
          ...(structuredError ? { error: structuredError } : {}),
          ...(isObject(errorData) && errorData.query_error === true ? { query_error: true as const } : {}),
          ...(recovery ? { recovery } : {}),
          ...(notice ? { idmm_notice: notice } : {}),
        },
      };
    }
    case 'tips': {
      const data = isObject(message.data) ? message.data : { content: message.data };
      const content = toDisplayText(data.content);
      const tipType = normalizeTipType(data.type);
      const structuredError =
        tipType === 'error'
          ? (normalizeAgentStreamError(data.error) ?? normalizeAgentStreamError({ ...data, message: content }))
          : undefined;
      const recovery = normalizeTruncatedTurnRecovery(data.recovery);
      const notice = normalizeIdmmDecisionNotice(data.idmm_notice);
      const userInputRequest = tipType !== 'error' ? normalizeAgentUserInputRequest(data.user_input_request) : undefined;
      const outcome = normalizeAgentTurnOutcome(data.turn_outcome);
      const startedAtMs = finiteNumber(data.started_at_ms);
      const finishedAtMs = finiteNumber(data.finished_at_ms);
      return {
        id: uuid(),
        type: 'tips',
        msg_id: message.msg_id,
        ...turnIdentity,
        position: 'center',
        conversation_id: message.conversation_id,
        created_at,
        content: {
          content,
          type: tipType,
          ...(structuredError ? { error: structuredError } : {}),
          ...(tipType === 'error' && data.query_error === true ? { query_error: true as const } : {}),
          ...(recovery ? { recovery } : {}),
          ...(notice ? { idmm_notice: notice } : {}),
          ...(userInputRequest && message.turn_id && message.msg_id ? { user_input_request: userInputRequest } : {}),
          ...(outcome ? { turn_outcome: outcome } : {}),
          ...(startedAtMs !== undefined && startedAtMs > 0 ? { started_at_ms: startedAtMs } : {}),
          ...(finishedAtMs !== undefined && finishedAtMs > 0 ? { finished_at_ms: finishedAtMs } : {}),
        },
      };
    }
    case 'text':
    case 'content':
    case 'user_content': {
      const data = message.data;
      const isRichData = isResponseTextData(data);
      const shouldReplace = message.replace === true || (isRichData && data.replace === true);
      const decision = isRichData ? normalizeIdmmDecisionExplanation(data.idmm_decision) : undefined;
      return {
        id: uuid(),
        type: 'text',
        msg_id: message.msg_id,
        ...turnIdentity,
        position: message.type === 'user_content' ? 'right' : 'left',
        conversation_id: message.conversation_id,
        created_at,
        content: isRichData
          ? {
              content: toDisplayText(data.content),
              cronMeta: normalizeCronMessageMeta(data.cronMeta),
              ...(shouldReplace ? { replace: true } : {}),
              ...(decision ? { idmm_decision: decision } : {}),
              ...normalizeTextContinuation(data),
              ...(message.stream_complete && !message.turn_id ? { standalone_projection: true as const } : {}),
              ...normalizeWireAgentMessageMetadata(data as Record<string, unknown>),
            }
          : {
              content: toDisplayText(data),
              ...(message.stream_complete && !message.turn_id ? { standalone_projection: true as const } : {}),
              ...(shouldReplace ? { replace: true } : {}),
            },
        ...(message.hidden && { hidden: true }),
      };
    }
    case 'tool_call': {
      const data = isObject(message.data) ? message.data : {};
      return {
        id: uuid(),
        type: 'tool_call',
        msg_id: message.msg_id,
        ...turnIdentity,
        conversation_id: message.conversation_id,
        position: 'left',
        created_at,
        content: normalizeToolCallContent(data),
      };
    }
    case 'tool_group': {
      return {
        type: 'tool_group',
        id: uuid(),
        msg_id: message.msg_id,
        ...turnIdentity,
        conversation_id: message.conversation_id,
        created_at,
        content: normalizeToolGroupContent(message.data),
      };
    }
    case 'agent_status': {
      return {
        id: uuid(),
        type: 'agent_status',
        msg_id: message.msg_id,
        ...turnIdentity,
        position: 'center',
        conversation_id: message.conversation_id,
        created_at,
        content: normalizeAgentStatusContent(message.data),
      };
    }
    case 'thinking': {
      const data = isObject(message.data) ? message.data : { content: message.data };
      const duration = finiteNumber(data.duration) ?? finiteNumber(data.duration_ms);
      return {
        id: uuid(),
        type: 'thinking',
        msg_id: message.msg_id,
        ...turnIdentity,
        position: 'left',
        conversation_id: message.conversation_id,
        created_at,
        content: {
          content: toDisplayText(data.content),
          ...(data.subject != null ? { subject: toDisplayText(data.subject) } : {}),
          ...(duration !== undefined ? { duration } : {}),
          status: normalizeThinkingStatus(data.status),
        },
      };
    }
    case 'start':
    case 'output_discarded':
    case 'finish':
    case 'thought':
    case 'skill_suggest':
    case 'cron_trigger':
    case 'info': // Stream retry notifications and similar transient agent updates
    case 'request_trace': // Transient request traces are not persisted or rendered.
      return undefined;
    case 'system':
      return undefined;
    default: {
      console.warn(
        `[transformMessage] Unsupported message type '${message.type}'. All non-standard message types should be pre-processed by respective AgentManagers.`
      );
      return undefined;
    }
  }
};

export function normalizeTextContinuation(value: Record<string, unknown>): Partial<IMessageText['content']> {
  if (typeof value.continuation_of_message_id !== 'string') return {};
  try { return { continuation_of_message_id: parseMessageId(value.continuation_of_message_id) }; }
  catch { return {}; }
}

const normalizeMessageStatus = (value: string | undefined): TMessage['status'] => {
  if (value === 'finish' || value === 'pending' || value === 'error' || value === 'work') return value;
  return 'finish';
};

export const transformUserCreatedEvent = (
  event: IUserMessageCreatedEvent,
  conversationId: ConversationId
): IMessageText | undefined => {
  if (event.hidden || event.conversation_id !== conversationId || !event.msg_id) return undefined;
  const decision = normalizeIdmmDecisionExplanation(event.idmm_decision);
  return {
    id: uuid(),
    type: 'text',
    msg_id: event.msg_id,
    position: 'right',
    status: normalizeMessageStatus(event.status),
    conversation_id: event.conversation_id,
    created_at: event.created_at,
    content: {
      content: event.content,
      ...(typeof event.display_at_ms === 'number' && Number.isFinite(event.display_at_ms) && event.display_at_ms > 0
        ? { display_at_ms: event.display_at_ms } : {}),
      ...(event.interaction ? { interaction: event.interaction } : {}),
      ...(decision ? { idmm_decision: decision } : {}),
    },
  };
};

/**
 * @description 将消息合并到消息列表中
 * */
export const composeMessage = (
  message: TMessage | undefined,
  list: TMessage[] | undefined,
  messageHandler: (type: 'update' | 'insert', message: TMessage) => void = () => {}
): TMessage[] => {
  if (!message) return list || [];
  if (!list?.length) {
    messageHandler('insert', message);
    return [message];
  }
  const last = list[list.length - 1];

  const updateMessage = (index: number, message: TMessage, change = true) => {
    message.id = list[index].id;
    list[index] = message;
    if (change) messageHandler('update', message);
    return list.slice();
  };
  const pushMessage = (message: TMessage) => {
    list.push(message);
    messageHandler('insert', message);
    return list.slice();
  };

  if (message.type === 'tool_group') {
    if (!Array.isArray(message.content)) return list;
    const remainingToolsMap = new Map(message.content.map((t) => [t.call_id, t] as const));
    if (remainingToolsMap.size === 0) return list;

    const updatesToReport: TMessage[] = [];

    const updatedList = list.map((existingMessage) => {
      if (existingMessage.type !== 'tool_group') return existingMessage;
      if (existingMessage.msg_id !== message.msg_id) return existingMessage;
      if (!existingMessage.content.length) return existingMessage;

      let didMergeIntoThisMessage = false;
      const new_content = existingMessage.content.map((tool) => {
        const newToolData = remainingToolsMap.get(tool.call_id);
        if (!newToolData) return tool;
        didMergeIntoThisMessage = true;
        remainingToolsMap.delete(tool.call_id);
        // Create new object instead of mutating original
        const merged = { ...tool, ...newToolData };
        const existingTerminal = ['Success', 'Error', 'Canceled'].includes(tool.status);
        if (existingTerminal) {
          return tool;
        }
        return merged;
      });

      if (!didMergeIntoThisMessage) return existingMessage;
      const updatedMessage = { ...existingMessage, content: new_content } as TMessage;
      updatesToReport.push(updatedMessage);
      return updatedMessage;
    });

    const didUpdateExisting = updatesToReport.length > 0;
    for (const updatedMessage of updatesToReport) {
      messageHandler('update', updatedMessage);
    }

    const baseList = didUpdateExisting ? updatedList : list;

    // If there are new tool calls, append them as a new tool_group message (without mutating inputs)
    if (remainingToolsMap.size > 0) {
      const newTools = Array.from(remainingToolsMap.values());
      const insertMessage = { ...message, content: newTools } as TMessage;
      messageHandler('insert', insertMessage);
      return baseList.concat(insertMessage);
    }
    // No new tools appended; return a new list only if something was updated
    return didUpdateExisting ? baseList : list;
  }

  // Handle Gemini tool_call message merging
  if (message.type === 'tool_call') {
    for (let i = 0, len = list.length; i < len; i++) {
      const msg = list[i];
      if (
        msg.type === 'tool_call' &&
        msg.msg_id === message.msg_id &&
        msg.content.call_id === message.content.call_id
      ) {
        const content = mergeToolCallContent(msg.content, message.content);
        return updateMessage(i, { ...msg, ...message, content });
      }
    }
    // If no existing tool call found, add new one
    return pushMessage(message);
  }

  // Reasoning has one canonical identity per Turn/model step. A provider can
  // resume that step after narration or a tool proposal, so reconcile by that
  // identity rather than creating duplicate rows at transcript boundaries.
  if (message.type === 'thinking') {
    if (message.msg_id) {
      for (let i = list.length - 1; i >= 0; i--) {
        const msg = list[i];
        if (msg.type !== 'thinking' || msg.msg_id !== message.msg_id) continue;

        const merged = {
          ...msg.content,
          content: msg.content.content + message.content.content,
          status: message.content.status,
          duration: message.content.duration,
          subject: message.content.subject || msg.content.subject,
        };
        return updateMessage(i, { ...msg, turn_id: message.turn_id ?? msg.turn_id, content: merged });
      }
    }
    return pushMessage(message);
  }

  if (last.msg_id !== message.msg_id || last.type !== message.type) {
    return pushMessage(message);
  }
  if (message.type === 'text' && last.type === 'text') {
    message.content = mergeTextMessageContent(last.content, message.content);
  }
  if (message.type === 'tips' && last.type === 'tips') {
    message.content = mergeTipsContent(last.content, message.content);
  }
  return updateMessage(list.length - 1, Object.assign({}, last, message));
};
