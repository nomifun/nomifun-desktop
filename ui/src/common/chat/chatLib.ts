/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  PlanUpdate,
  PersistedToolArtifact,
} from '@/common/types/platform/toolCallTypes';
import type { IKnowledgeWritebackEvent, IResponseMessage, IUserMessageCreatedEvent } from '../adapter/ipcBridge';
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
import { isAbsoluteLocalPath, isFileUri } from '../utils/localPath';

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
  | 'plan'
  | 'thinking'
  | 'available_commands';

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

export type KnowledgeWritebackStatus =
  | 'started'
  | 'extracting'
  | 'writing'
  | 'written'
  | 'partial'
  | 'failed'
  | 'no_candidate'
  | 'no_completer'
  | 'disabled'
  | 'interrupted';

export type KnowledgeWritebackFile = {
  kb_id?: KnowledgeBaseId | null;
  rel_path?: string | null;
};

export type KnowledgeWritebackFailure = {
  kb_id?: KnowledgeBaseId | null;
  rel_path?: string | null;
  error?: string;
};

export type KnowledgeWritebackState = {
  status: KnowledgeWritebackStatus;
  attempt_id?: string;
  attempt_generation?: number;
  started_at?: number;
  updated_at?: number;
  finished_at?: number | null;
  retryable?: boolean;
  candidates?: number;
  written?: KnowledgeWritebackFile[];
  failures?: KnowledgeWritebackFailure[];
  interrupted_at?: number;
};

export type IMessageText = IMessage<
  'text',
  {
    content: string;
    /** Backend explicitly replaced the accumulated text for this msg_id. */
    replace?: boolean;
    cronMeta?: CronMessageMeta;
    /** True when this reply was sent by another Agent participating in the task. */
    agentMessage?: boolean;
    senderName?: string;
    senderAgentType?: string;
    /** Sender Agent's conversation id — lets the renderer resolve preset avatars via conversation extras. */
    senderConversationId?: ConversationId;
    /** Turn-final knowledge write-back state, rendered under the assistant message. */
    knowledge_writeback?: KnowledgeWritebackState;
  }
>;

export type AgentErrorOwnership = 'nomifun' | 'user_agent' | 'user_llm_provider' | 'unknown_upstream';

export type AgentErrorResolutionKind =
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

export type AgentErrorResolutionTarget = 'provider_settings' | 'agent_settings' | 'new_conversation' | 'feedback';

export type AgentErrorResolution = {
  kind: AgentErrorResolutionKind;
  target?: AgentErrorResolutionTarget;
};

export type AgentStreamErrorInfo = {
  message: string;
  code?: string;
  ownership?: AgentErrorOwnership;
  detail?: string;
  workspacePath?: string;
  retryable?: boolean;
  feedback_recommended?: boolean;
  resolution?: AgentErrorResolution;
};

export type TruncatedTurnFailureCode = 'output_truncated' | 'turn_requests_exhausted';

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
    recovery?: TruncatedTurnRecovery;
  }
>;

export type IMessageToolCall = IMessage<
  'tool_call',
  {
    call_id: string;
    name: string;
    /**
     * Provider arguments when they were decoded as a JSON object. Local
     * Pre-execution validation failures preserve the rejected object so users
     * can inspect exactly what the model sent.
     */
    args?: Record<string, unknown> | null;
    error?: string;
    status?: 'running' | 'completed' | 'error';
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
    render_output_as_markdown: boolean;
    result_display?:
      | string
      | {
          file_diff: string;
          file_name: string;
        }
      | {
          img_url: string;
          relative_path: string;
        };
    status: 'Executing' | 'Success' | 'Error' | 'Canceled' | 'Pending';
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
  }
>;

type ResponseTextData = {
  content: unknown;
  replace?: boolean;
  cronMeta?: CronMessageMeta;
  knowledge_writeback?: unknown;
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
  'started',
  'extracting',
  'writing',
  'written',
  'partial',
  'failed',
  'no_candidate',
  'no_completer',
  'disabled',
  'interrupted',
]);

const normalizeKnowledgeWritebackFiles = (value: unknown): KnowledgeWritebackFile[] | undefined => {
  if (!Array.isArray(value)) return undefined;
  return value
    .filter(isObject)
    .map((file) => ({
      ...(file.kb_id === null
        ? { kb_id: null }
        : typeof file.kb_id === 'string'
          ? { kb_id: parseKnowledgeBaseId(file.kb_id) }
          : {}),
      ...(typeof file.rel_path === 'string' || file.rel_path === null ? { rel_path: file.rel_path } : {}),
    }));
};

const normalizeKnowledgeWritebackFailures = (value: unknown): KnowledgeWritebackFailure[] | undefined => {
  if (!Array.isArray(value)) return undefined;
  return value
    .filter(isObject)
    .map((failure) => ({
      ...(failure.kb_id === null
        ? { kb_id: null }
        : typeof failure.kb_id === 'string'
          ? { kb_id: parseKnowledgeBaseId(failure.kb_id) }
          : {}),
      ...(typeof failure.rel_path === 'string' || failure.rel_path === null ? { rel_path: failure.rel_path } : {}),
      ...(typeof failure.error === 'string' ? { error: failure.error } : {}),
    }));
};

export const normalizeKnowledgeWritebackState = (value: unknown): KnowledgeWritebackState | undefined => {
  if (!isObject(value) || typeof value.status !== 'string') return undefined;
  if (!KNOWLEDGE_WRITEBACK_STATUSES.has(value.status as KnowledgeWritebackStatus)) return undefined;
  const written = normalizeKnowledgeWritebackFiles(value.written);
  const failures = normalizeKnowledgeWritebackFailures(value.failures);
  return {
    status: value.status as KnowledgeWritebackStatus,
    ...(typeof value.attempt_id === 'string' ? { attempt_id: value.attempt_id } : {}),
    ...(Number.isSafeInteger(value.attempt_generation) && (value.attempt_generation as number) >= 0
      ? { attempt_generation: value.attempt_generation as number }
      : {}),
    ...(typeof value.started_at === 'number' ? { started_at: value.started_at } : {}),
    ...(typeof value.updated_at === 'number' ? { updated_at: value.updated_at } : {}),
    ...(typeof value.finished_at === 'number' || value.finished_at === null ? { finished_at: value.finished_at } : {}),
    ...(typeof value.retryable === 'boolean' ? { retryable: value.retryable } : {}),
    ...(typeof value.candidates === 'number' ? { candidates: value.candidates } : {}),
    ...(written ? { written } : {}),
    ...(failures ? { failures } : {}),
    ...(typeof value.interrupted_at === 'number' ? { interrupted_at: value.interrupted_at } : {}),
  };
};

const knowledgeWritebackTime = (state: KnowledgeWritebackState | undefined): number | undefined => {
  if (!state) return undefined;
  return state.updated_at ?? state.finished_at ?? state.interrupted_at ?? state.started_at;
};

const RUNNING_KNOWLEDGE_WRITEBACK_STATUSES = new Set<KnowledgeWritebackStatus>([
  'started',
  'extracting',
  'writing',
]);

const TERMINAL_KNOWLEDGE_WRITEBACK_STATUSES = new Set<KnowledgeWritebackStatus>([
  'written',
  'partial',
  'failed',
  'no_candidate',
  'no_completer',
  'disabled',
  'interrupted',
]);

export const preferKnowledgeWritebackState = (
  existing: KnowledgeWritebackState | undefined,
  incoming: KnowledgeWritebackState | undefined
): KnowledgeWritebackState | undefined => {
  if (!existing) return incoming;
  if (!incoming) return existing;

  const existingAttempt = existing.attempt_id;
  const incomingAttempt = incoming.attempt_id;

  // An identified attempt is authoritative over a legacy/unidentified state.
  // This also prevents an old persisted projection from replacing the first
  // frame of a manual retry.
  if (existingAttempt && !incomingAttempt) return existing;
  if (!existingAttempt && incomingAttempt) return incoming;

  if (existingAttempt && incomingAttempt && existingAttempt !== incomingAttempt) {
    const existingGeneration = existing.attempt_generation;
    const incomingGeneration = incoming.attempt_generation;
    if (existingGeneration !== undefined || incomingGeneration !== undefined) {
      if (existingGeneration === undefined) return incoming;
      if (incomingGeneration === undefined) return existing;
      if (incomingGeneration !== existingGeneration) {
        return incomingGeneration > existingGeneration ? incoming : existing;
      }
    }
    // Attempt IDs are opaque; started_at is the generation ordering contract.
    // Fall back to each attempt's best event timestamp only for legacy frames
    // that omitted started_at.
    const existingStarted = existing.started_at ?? knowledgeWritebackTime(existing);
    const incomingStarted = incoming.started_at ?? knowledgeWritebackTime(incoming);
    if (existingStarted === undefined && incomingStarted === undefined) return incoming;
    if (existingStarted === undefined) return incoming;
    if (incomingStarted === undefined) return existing;
    if (incomingStarted !== existingStarted) {
      return incomingStarted > existingStarted ? incoming : existing;
    }
    const existingTime = knowledgeWritebackTime(existing);
    const incomingTime = knowledgeWritebackTime(incoming);
    if (existingTime === undefined && incomingTime === undefined) return incoming;
    if (existingTime === undefined) return incoming;
    if (incomingTime === undefined) return existing;
    return incomingTime >= existingTime ? incoming : existing;
  }

  // Within one attempt, terminal states are monotonic. A delayed extracting or
  // writing frame must never resurrect a completed/failed attempt.
  const existingTerminal = TERMINAL_KNOWLEDGE_WRITEBACK_STATUSES.has(existing.status);
  const incomingTerminal = TERMINAL_KNOWLEDGE_WRITEBACK_STATUSES.has(incoming.status);
  if (existingTerminal && RUNNING_KNOWLEDGE_WRITEBACK_STATUSES.has(incoming.status)) {
    return existing;
  }
  if (incomingTerminal && RUNNING_KNOWLEDGE_WRITEBACK_STATUSES.has(existing.status)) {
    return incoming;
  }

  const existingTime = knowledgeWritebackTime(existing);
  const incomingTime = knowledgeWritebackTime(incoming);
  if (existingTime === undefined && incomingTime === undefined) return incoming;
  if (existingTime === undefined) return incoming;
  if (incomingTime === undefined) return existing;
  return incomingTime >= existingTime ? incoming : existing;
};

const isResponseTextData = (data: unknown): data is ResponseTextData =>
  typeof data === 'object' &&
  data !== null &&
  'content' in data &&
  !Array.isArray(data);

export const isTextContentReplacement = (content: IMessageText['content'] | undefined): boolean =>
  content?.replace === true;

export const mergeTextMessageContent = (
  existing: IMessageText['content'],
  incoming: IMessageText['content']
): IMessageText['content'] => {
  const { replace: _existingReplace, knowledge_writeback: existingWriteback, ...existingRest } = existing;
  const { replace: incomingReplace, knowledge_writeback: incomingWriteback, ...incomingRest } = incoming;
  const knowledgeWriteback = preferKnowledgeWritebackState(existingWriteback, incomingWriteback);

  return {
    ...existingRest,
    ...incomingRest,
    content: incomingReplace ? incoming.content : existing.content + incoming.content,
    ...(incomingReplace ? { replace: true } : {}),
    ...(knowledgeWriteback ? { knowledge_writeback: knowledgeWriteback } : {}),
  };
};

export const preferTextMessageVersion = (primary: IMessageText, secondary: IMessageText): IMessageText => {
  const primaryIsReplace = isTextContentReplacement(primary.content);
  const secondaryIsReplace = isTextContentReplacement(secondary.content);
  const mergePreferredWriteback = (preferred: IMessageText, fallback: IMessageText): IMessageText => {
    const knowledgeWriteback = preferKnowledgeWritebackState(
      fallback.content.knowledge_writeback,
      preferred.content.knowledge_writeback
    );
    if (!knowledgeWriteback) return preferred;
    return {
      ...preferred,
      content: {
        ...preferred.content,
        knowledge_writeback: knowledgeWriteback,
      },
    };
  };

  if (primaryIsReplace !== secondaryIsReplace) {
    return primaryIsReplace ? mergePreferredWriteback(primary, secondary) : mergePreferredWriteback(secondary, primary);
  }

  return secondary.content.content.length > primary.content.content.length
    ? mergePreferredWriteback(secondary, primary)
    : mergePreferredWriteback(primary, secondary);
};

export type IMessagePlan = IMessage<
  'plan',
  {
    session_id: string;
    entries: PlanUpdate['update']['entries'];
  }
>;

export type IMessageThinking = IMessage<
  'thinking',
  {
    content: string;
    subject?: string;
    duration?: number;
    status: 'thinking' | 'done';
  }
>;

// Available commands advertised by the agent runtime.
export type AvailableCommand = {
  name: string;
  description: string;
  hint?: string;
};

export type IMessageAvailableCommands = IMessage<
  'available_commands',
  {
    commands: AvailableCommand[];
  }
>;

// eslint-disable-next-line max-len
export type TMessage =
  | IMessageText
  | IMessageTips
  | IMessageToolCall
  | IMessageToolGroup
  | IMessageAgentStatus
  | IMessagePlan
  | IMessageThinking
  | IMessageAvailableCommands;

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

export const normalizeAgentErrorResolution = (value: unknown): AgentErrorResolution | undefined => {
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
  const retryable = typeof value.retryable === 'boolean' ? value.retryable : undefined;
  const feedback_recommended = typeof value.feedback_recommended === 'boolean' ? value.feedback_recommended : undefined;
  const resolution = normalizeAgentErrorResolution(value.resolution);

  if (
    !code &&
    !ownership &&
    !detail &&
    !workspacePath &&
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

const normalizeToolGroupResultDisplay = (
  value: unknown
): IMessageToolGroup['content'][number]['result_display'] | undefined => {
  if (value == null) return undefined;
  if (typeof value === 'string') return value;
  if (!isObject(value)) return toDisplayText(value);

  if ('file_diff' in value || 'file_name' in value) {
    return {
      file_diff: toDisplayText(value.file_diff),
      file_name: toDisplayText(value.file_name),
    };
  }
  if ('img_url' in value || 'relative_path' in value) {
    return {
      img_url: toDisplayText(value.img_url),
      relative_path: toDisplayText(value.relative_path),
    };
  }

  return toDisplayText(value);
};

const LEGACY_TOOL_GROUP_ARTIFACT_ERROR =
  'Legacy image result was not backed by a committed artifact receipt';

export const normalizeToolGroupContent = (value: unknown): IMessageToolGroup['content'] => {
  if (!Array.isArray(value)) return [];

  return value
    .filter(isObject)
    .map((item) => {
      const resultDisplay = normalizeToolGroupResultDisplay(item.result_display);
      const status = normalizeToolGroupStatus(item.status);
      const description = toDisplayText(item.description);
      // ToolGroupEntry has no receipt or 2PC-marker fields. Historical
      // `result_display.img_url` therefore cannot prove delivery and must be
      // downgraded at message admission, before process summaries can render a
      // green state. Verified outputs use the detailed ToolCall carrier.
      const unverifiedLegacyImage =
        isObject(resultDisplay) &&
        'img_url' in resultDisplay &&
        Boolean(optionalDisplayText(resultDisplay.img_url));
      return {
        call_id: optionalDisplayText(item.call_id) ?? optionalDisplayText(item.id) ?? uuid(),
        description:
          unverifiedLegacyImage && status === 'Success'
            ? description
              ? `${description}: ${LEGACY_TOOL_GROUP_ARTIFACT_ERROR}`
              : LEGACY_TOOL_GROUP_ARTIFACT_ERROR
            : description,
        name: toDisplayText(item.name, 'Tool'),
        render_output_as_markdown:
          typeof item.render_output_as_markdown === 'boolean' ? item.render_output_as_markdown : false,
        status: unverifiedLegacyImage && status === 'Success' ? 'Error' : status,
        ...(!unverifiedLegacyImage && resultDisplay !== undefined
          ? { result_display: resultDisplay }
          : {}),
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
    data.status === 'running' || data.status === 'completed' || data.status === 'error'
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
    artifacts: invalidTerminalClaim ? [] : artifacts,
  } as IMessageToolCall['content'];
};

const normalizeAgentStatusContent = (value: unknown): IMessageAgentStatus['content'] => {
  const data = isObject(value) ? value : {};
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
          ...(recovery ? { recovery } : {}),
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
          ...(recovery ? { recovery } : {}),
        },
      };
    }
    case 'text':
    case 'content':
    case 'user_content': {
      const data = message.data;
      const isRichData = isResponseTextData(data);
      const shouldReplace = message.replace === true || (isRichData && data.replace === true);
      const persistedWriteback = isRichData ? normalizeKnowledgeWritebackState(data.knowledge_writeback) : undefined;
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
              ...(persistedWriteback ? { knowledge_writeback: persistedWriteback } : {}),
              ...normalizeWireAgentMessageMetadata(data as Record<string, unknown>),
            }
          : {
              content: toDisplayText(data),
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
    case 'plan': {
      return {
        id: uuid(),
        type: 'plan',
        msg_id: message.msg_id,
        ...turnIdentity,
        position: 'left',
        conversation_id: message.conversation_id,
        created_at,
        status: message.status,
        content: message.data as any,
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
    // Disabled: available_commands messages are too noisy and distracting in the chat UI
    case 'available_commands':
      return undefined;
    case 'start':
    case 'output_discarded':
    case 'finish':
    case 'thought':
    case 'skill_suggest':
    case 'cron_trigger':
    case 'info': // Stream retry notifications and similar transient agent updates
    case 'system': // Cron system responses, ignored
    case 'request_trace': // Request trace events, logged to F12 console (not persisted)
      return undefined;
    default: {
      console.warn(
        `[transformMessage] Unsupported message type '${message.type}'. All non-standard message types should be pre-processed by respective AgentManagers.`
      );
      return undefined;
    }
  }
};

export const transformKnowledgeWritebackEvent = (event: IKnowledgeWritebackEvent): IMessageText | undefined => {
  if (!event.msg_id) return undefined;
  return {
    id: uuid(),
    type: 'text',
    msg_id: event.msg_id,
    position: 'left',
    conversation_id: event.conversation_id,
    content: {
      content: '',
      knowledge_writeback: {
        status: event.status,
        attempt_id: event.attempt_id,
        attempt_generation: event.attempt_generation,
        started_at: event.started_at,
        updated_at: event.updated_at,
        finished_at: event.finished_at,
        retryable: event.retryable,
        candidates: event.candidates,
        written: event.written,
        failures: event.failures,
      },
    },
  };
};

const normalizeMessageStatus = (value: string | undefined): TMessage['status'] => {
  if (value === 'finish' || value === 'pending' || value === 'error' || value === 'work') return value;
  return 'finish';
};

export const transformUserCreatedEvent = (
  event: IUserMessageCreatedEvent,
  conversationId: ConversationId
): IMessageText | undefined => {
  if (event.hidden || event.conversation_id !== conversationId || !event.msg_id) return undefined;
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
        if (
          ['Success', 'Error', 'Canceled'].includes(newToolData.status) &&
          !Object.prototype.hasOwnProperty.call(newToolData, 'result_display')
        ) {
          // A provisional result retained from Executing is not proof of a
          // terminal output. The terminal frame must carry it explicitly.
          merged.result_display = undefined;
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

  if (message.type === 'plan') {
    for (let i = 0, len = list.length; i < len; i++) {
      const msg = list[i];
      if (msg.type === 'plan' && msg.content.session_id === message.content.session_id) {
        // Create new object instead of mutating original
        const merged = { ...msg.content, ...message.content };
        return updateMessage(i, { ...msg, content: merged });
      }
    }
    return pushMessage(message);
    // If no existing plan found, add new one
  }

  // Handle thinking message merging — only merge contiguous streaming chunks
  if (message.type === 'thinking') {
    if (message.content.status === 'done') {
      for (let i = list.length - 1; i >= 0; i--) {
        const msg = list[i];
        if (msg.type !== 'thinking' || msg.msg_id !== message.msg_id) continue;

        const merged = {
          ...msg.content,
          status: 'done' as const,
          duration: message.content.duration,
          subject: message.content.subject || msg.content.subject,
        };
        return updateMessage(i, { ...msg, content: merged });
      }
    }

    if (last.type === 'thinking' && last.msg_id === message.msg_id) {
      // Otherwise append content
      const merged = {
        ...last.content,
        content: last.content.content + message.content.content,
        subject: message.content.subject || last.content.subject,
      };
      return updateMessage(list.length - 1, { ...last, content: merged });
    }
    return pushMessage(message);
  }

  if (last.msg_id !== message.msg_id || last.type !== message.type) {
    return pushMessage(message);
  }
  if (message.type === 'text' && last.type === 'text') {
    message.content = mergeTextMessageContent(last.content, message.content);
  }
  return updateMessage(list.length - 1, Object.assign({}, last, message));
};
