/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * IPC Bridge → HTTP/WS adapter.
 *
 * This file replaces the original IPC bridge calls with HTTP REST and WebSocket
 * calls routed to nomicore. Electron-native operations (window controls,
 * native dialogs, auto-update, devtools, zoom, deep links) remain as IPC.
 */

import type { ConfirmationCorrelationId, IConfirmation } from '@/common/chat/chatLib';
import { bridge } from '@/platform';
import type { McpConnectionTestRequest } from './mcpRequest';
import {
  noopEmitter,
  shellEmitter,
  shellProvider,
  stubShellProvider,
  subscribeDeepLink,
  subscribeWebuiStatus,
  subscribeWindowMaximized,
  tauriGetPath,
  tauriGetZoom,
  tauriIsAutostartEnabled,
  tauriOpenDialog,
  tauriRelaunch,
  tauriSendNotification,
  tauriSetAutostart,
  tauriSetKeepAwake,
  tauriSetTrayLabels,
  tauriSetZoom,
  tauriWebuiGetStatus,
  tauriWebuiStart,
  tauriWebuiStop,
  tauriRelayPairingBootstrap,
  tauriRelayPairingDisconnect,
  tauriRelayPairingGetStatus,
  tauriRelayPairingRestart,
  tauriRelayPairingStop,
  type TauriRelayPairingBootstrapRequest,
  type TauriRelayPairingStatus,
  tauriWindowClose,
  tauriWindowIsMaximized,
  tauriWindowMaximize,
  tauriWindowMinimize,
  tauriWindowToggleMaximize,
  tauriWindowUnmaximize,
  type ShellOpenDialogOptions,
} from './tauriShell';
import {
  autoUpdateStatusEmitter,
  tauriUpdateCheck,
  tauriUpdateCurrentVersion,
  tauriUpdateDownload,
  tauriUpdatePackageSnapshot,
  tauriUpdateInstallAndRelaunch,
} from './tauriUpdater';
import type {
  ICssTheme,
  IMcpServer,
  ISessionMcpServer,
  TChatConversation,
  TProviderWithModel,
} from '../config/storage';
import type {
  CreatePresetRequest,
  CreatePresetTagRequest,
  ImportPresetsRequest,
  ImportPresetsResult,
  Preset,
  PresetReference,
  PresetTag,
  ResolvePresetRequest,
  ResolvedPresetSnapshot,
  SetPresetStateRequest,
  UpdatePresetRequest,
  UpdatePresetTagRequest,
} from '../types/agent/presetTypes';
import {
  parsePresetReference,
  parsePresetTagKey,
} from '../types/agent/presetTypes';
import type { PreviewHistoryTarget, PreviewSnapshotInfo, PreviewUrlResponse } from '../types/office/preview';
import { parsePresetTagId, parsePreviewSnapshotId } from '../types/ids';
import {
  fromProviderResponse,
  toCreateProviderRequest,
  toUpdateProviderRequest,
  type CreateProviderInput,
  type FetchModelsAnonymousRequest,
  type FetchModelsResponse,
  type ProviderResponse,
  type ProviderHealthCheckRequest,
  type ProviderHealthCheckResponse,
  type UpdateProviderRequest,
} from '../types/provider/providerApi';
import type {
  ProbeProviderConnectionAnonymousRequest,
  ProbeProviderConnectionRequest,
  ProbeProviderConnectionResponse,
} from '../types/provider/providerProbe';
import type {
  ModelProtocolManifestRequest,
  ModelProtocolManifestResponse,
} from '../types/provider/modelProtocolManifest';
import type {
  CheckManagedModelHealthRequest,
  ManagedModel,
  ManagedModelHealthBatchResult,
  ManagedModelHealthResult,
  ManagedModelServiceStatus,
  SetManagedModelEnabledRequest,
  SetManagedModelServiceEnabledRequest,
} from '../types/provider/managedModelService';
import type {
  ProviderModelKeyRequest,
  ProviderModelResponse,
  SaveProviderModelRequest,
} from '../types/provider/providerModel';
import type {
  ProviderConnectionResponse,
  SaveProviderConnectionRequest,
} from '../types/provider/providerConnection';
import type { KnowledgeRetrievalConfig as ApiKnowledgeRetrievalConfig } from '../protocolBindings/KnowledgeRetrievalConfig';
import type { RelocateKnowledgeEntryRequest as ApiRelocateKnowledgeEntryRequest } from '../protocolBindings/RelocateKnowledgeEntryRequest';
import type { RelocateKnowledgeEntryResponse as ApiRelocateKnowledgeEntryResponse } from '../protocolBindings/RelocateKnowledgeEntryResponse';
import type {
  TAdoptExecutionStepOutput,
  TAdjustAgentExecution,
  TAddExecutionSteps,
  TAgentExecution,
  TAgentExecutionDetail,
  TAgentExecutionEvent,
  TAgentExecutionEventsQuery,
  TAnswerExecutionDecision,
  TConfigureExecutionStep,
  TCreateAgentExecution,
  TDecisionPolicy,
  TDelegationPolicy,
  TExecutionModelPool,
  TExecutionAttempt,
  TExecutionParticipant,
  TExecutionStep,
  TExecutionStepDependency,
  TReassignExecutionStep,
  TRenameAgentExecution,
  TReplanAgentExecution,
  TRetryExecutionStep,
  TSteerExecutionStep,
  TUpdateExecutionStep,
  TVersionedAgentExecutionCommand,
} from '../types/agentExecution/agentExecutionTypes';
import type {
  TAgentExecutionChangedEvent,
  TAgentExecutionLeadThinkingEvent,
} from '../types/agentExecution/agentExecutionEvents';
import type {
  TAgentExecutionTemplate,
  TAgentExecutionTemplateDetail,
  TAgentExecutionTemplateParticipant,
  TCreateAgentExecutionTemplate,
  TCreateExecutionFromTemplate,
  TUpdateAgentExecutionTemplate,
} from '../types/agentExecution/agentExecutionTemplateTypes';
import type {
  UpdateCheckRequest,
  UpdateCheckResult,
  UpdateDownloadProgressEvent,
  UpdateDownloadRequest,
  UpdateDownloadResult,
  UpdateReleaseInfo,
} from '../update/updateTypes';
import {
  fromApiConversation,
  fromApiPaginatedConversations,
  fromApiResolvedPresetSnapshot,
  toApiModelOptional,
} from './apiModelMapper';
import {
  parseAgentId,
  parseAttachmentId,
  parseChannelPluginId,
  parseChannelSessionId,
  parseChannelUserId,
  parseCompanionEventId,
  parseCompanionId,
  parseCompanionMemoryId,
  parseCompanionSessionWindowId,
  parseCompanionSkillId,
  parseConversationId,
  parseCronJobId,
  parseCronJobRunId,
  parseExecutionAttemptId,
  parseExecutionId,
  parseExecutionParticipantId,
  parseExecutionStepId,
  parseExecutionTemplateId,
  parseExecutionTemplateParticipantId,
  parseFigureId,
  parseIdmmInterventionId,
  parseKnowledgeBaseId,
  parseKnowledgeEntryId,
  parseKnowledgeSourceId,
  parseKnowledgeSourceItemId,
  parseMessageId,
  parseMcpServerId,
  parseOptionalEntityId,
  parseProviderId,
  parseCsAgentId,
  parseCsDialogueId,
  parseCsMessageId,
  parseCsNoteId,
  parseRequirementId,
  parseMiniAppId,
  parseSshHostId,
  parseSkillPatternId,
  parseTerminalId,
  parseUserId,
  parseWebhookId,
  type AgentId,
  type AttachmentId,
  type ChannelPluginId,
  type ConversationId,
  type CronJobId,
  type CronJobRunId,
  type CompanionEventId,
  type CompanionId,
  type CompanionMemoryId,
  type CompanionSessionWindowId,
  type CompanionSkillId,
  type FigureId,
  type IdmmInterventionId,
  type ExecutionAttemptId,
  type ExecutionId,
  type ExecutionStepId,
  type ExecutionTemplateId,
  type McpServerId,
  type MessageId,
  type MiniAppId,
  type ProviderId,
  type CsAgentId,
  type CsDialogueId,
  type CsMessageId,
  type CsNoteId,
  type ChannelUserId,
  type KnowledgeBaseId,
  type KnowledgeEntryId,
  type KnowledgeSourceId,
  type KnowledgeSourceItemId,
  type RequirementId,
  type SshHostId,
  type SkillPatternId,
  type TerminalId,
  type WebhookId,
} from '../types/ids';
import {
  httpDelete,
  httpGet,
  httpPatch,
  httpPost,
  httpPut,
  httpRequest,
  isBackendHttpError,
  stubProvider,
  withResponseMap,
  wsEmitter,
  wsMappedEmitter,
} from './httpBridge';

export { browserSession } from '@/common/browser/browserSession';
export type {
  BrowserCloseResult,
  BrowserIdentityMode,
  BrowserLaneLifecycleState,
  BrowserResourcePressureState,
  IBrowserCapacityOverview,
  IBrowserInventoryChangedEvent,
  IBrowserLane,
  IBrowserLaneIdentity,
  IBrowserLaneOwner,
  IBrowserLaneQueue,
  IBrowserOverview,
  IBrowserTab,
} from '@/common/browser/browserTypes';
import {
  parseConversationArtifactId,
  type ConversationArtifactId,
} from '../types/conversationArtifact';
import { fromApiSearchResult, type ApiMessageSearchItem } from './searchMapper';
import { fromBackendCompareResult, type RawCompareResult } from './fileSnapshotMapper';
import {
  fromApiStoredMessage,
  type StoredMessageResponse,
} from './storedMessageMapper';
import {
  absoluteToRelativePath,
  fromBackendWorkspaceFlatFiles,
  fromBackendWorkspaceList,
  type RawWorkspaceFlatFile,
} from './workspaceMapper';

// ---------------------------------------------------------------------------
// Shell — routed to POST /api/shell/*
// ---------------------------------------------------------------------------

export const shell = {
  openFile: httpPost<void, string>('/api/shell/open-file', (file_path) => ({
    file_path,
  })),
  showItemInFolder: httpPost<void, string>('/api/shell/show-item-in-folder', (file_path) => ({ file_path })),
  openExternal: httpPost<void, string>('/api/shell/open-external', (url) => ({
    url,
  })),
  checkToolInstalled: httpPost<boolean, { tool: string }>('/api/shell/check-tool-installed'),
  openFolderWith: httpPost<void, { folder_path: string; tool: 'vscode' | 'terminal' | 'explorer' }>(
    '/api/shell/open-folder-with'
  ),
};

// ---------------------------------------------------------------------------
// Presets — reusable launch configuration catalog
// ---------------------------------------------------------------------------

const fromApiPreset = (preset: Preset): Preset => {
  if (Object.prototype.hasOwnProperty.call(preset, 'id')) {
    throw new TypeError('Preset response legacy field "id" is not accepted; use "preset_id"');
  }
  return {
    ...preset,
    preset_id: parsePresetReference(preset.preset_id, preset.source),
    model_preferences: preset.model_preferences.map((model) => ({
      ...model,
      ...(model.provider_id == null ? {} : { provider_id: parseProviderId(model.provider_id) }),
    })),
    knowledge_bases: preset.knowledge_bases.map((binding) => ({
      ...binding,
      knowledge_base_id: parseKnowledgeBaseId(binding.knowledge_base_id),
    })),
    audience_tag_ids: preset.audience_tag_ids.map(parsePresetTagId),
    scenario_tag_ids: preset.scenario_tag_ids.map(parsePresetTagId),
  };
};

const fromApiPresetTag = (tag: PresetTag): PresetTag => ({
  ...tag,
  preset_tag_id: parsePresetTagId(tag.preset_tag_id),
  key: parsePresetTagKey(tag.key),
});

export const presets = {
  list: withResponseMap(httpGet<Preset[], void>('/api/presets'), (items) => items.map(fromApiPreset)),
  get: withResponseMap(
    httpGet<Preset, { preset_id: Preset['preset_id'] }>(
      (p) => `/api/presets/${encodeURIComponent(p.preset_id)}`
    ),
    fromApiPreset
  ),
  create: withResponseMap(httpPost<Preset, CreatePresetRequest>('/api/presets'), fromApiPreset),
  update: withResponseMap(httpPut<Preset, { preset_id: Preset['preset_id'] } & UpdatePresetRequest>(
    (p) => `/api/presets/${encodeURIComponent(p.preset_id)}`,
    (p) => {
      const { preset_id: _presetId, ...body } = p;
      return body;
    }
  ), fromApiPreset),
  delete: httpDelete<void, { preset_id: Preset['preset_id'] }>(
    (p) => `/api/presets/${encodeURIComponent(p.preset_id)}`
  ),
  setState: withResponseMap(httpPatch<Preset, SetPresetStateRequest>(
    (p) => `/api/presets/${encodeURIComponent(p.preset_id)}/state`,
    (p) => {
      const { preset_id: _presetId, ...body } = p;
      return body;
    }
  ), fromApiPreset),
  resolve: withResponseMap(httpPost<ResolvedPresetSnapshot, ResolvePresetRequest>(
    (p) => `/api/presets/${encodeURIComponent(p.preset_id)}/resolve`,
    (p) => {
      const { preset_id: _presetId, ...body } = p;
      return body;
    }
  ), fromApiResolvedPresetSnapshot),
  import: httpPost<ImportPresetsResult, ImportPresetsRequest>('/api/presets/import'),
};

// ---------------------------------------------------------------------------
// Preset Tags
// ---------------------------------------------------------------------------

export const presetTags = {
  list: withResponseMap(httpGet<PresetTag[], void>('/api/preset-tags'), (items) => items.map(fromApiPresetTag)),
  create: withResponseMap(httpPost<PresetTag, CreatePresetTagRequest>('/api/preset-tags'), fromApiPresetTag),
  update: withResponseMap(httpPut<PresetTag, UpdatePresetTagRequest>(
    (p) => `/api/preset-tags/${encodeURIComponent(p.preset_tag_id)}`,
    (p) => {
      const { preset_tag_id: _presetTagId, ...body } = p;
      return body;
    }
  ), fromApiPresetTag),
  delete: httpDelete<void, { preset_tag_id: PresetTag['preset_tag_id'] }>(
    (p) => `/api/preset-tags/${encodeURIComponent(p.preset_tag_id)}`
  ),
};

// ---------------------------------------------------------------------------
// Conversation — REST + WS
// ---------------------------------------------------------------------------

const fromApiSendMessageResult = (result: ISendMessageResult): ISendMessageResult => ({
  ...result,
  msg_id: parseMessageId(result.msg_id),
  // Current servers always send an explicit boolean. A legacy/malformed
  // response without replay authority must fail closed as an accepted replay:
  // authoritative GET reconciliation may reopen a running turn, but the client
  // may not manufacture a fresh one.
  replayed: result.replayed !== false,
  completed: result.completed === true,
  result_ok: result.result_ok ?? null,
  result_text: result.result_text ?? null,
  result_error: result.result_error ?? null,
  result_error_code: result.result_error_code ?? null,
  result_error_retryable: result.result_error_retryable ?? null,
});

const requireConversationIdempotencyKey = (value: unknown): string => {
  if (typeof value !== 'string' || value.length === 0) {
    throw new Error('conversation mutation requires a stable idempotency key');
  }
  return value;
};

type ConversationArtifactResponseFor<T extends IConversationArtifact> = T extends IConversationArtifact
  ? Omit<T, 'conversation_artifact_id'> & {
      conversation_artifact_id: unknown;
      artifact_id?: never;
      id?: never;
    }
  : never;

type ConversationArtifactResponse = ConversationArtifactResponseFor<IConversationArtifact>;

const fromApiConversationArtifact = (
  artifact: ConversationArtifactResponse
): IConversationArtifact => {
  if (
    Object.prototype.hasOwnProperty.call(artifact, 'id') ||
    Object.prototype.hasOwnProperty.call(artifact, 'artifact_id')
  ) {
    throw new TypeError(
      'conversation artifact wire payload must use conversation_artifact_id, not id or artifact_id'
    );
  }
  const common = {
    ...artifact,
    conversation_artifact_id: parseConversationArtifactId(artifact.conversation_artifact_id),
    conversation_id: parseConversationId(artifact.conversation_id),
    cron_job_id: artifact.cron_job_id == null ? undefined : parseCronJobId(artifact.cron_job_id),
  };
  if (artifact.kind === 'cron_trigger') {
    return {
      ...common,
      kind: artifact.kind,
      payload: {
        ...artifact.payload,
        cron_job_id: parseCronJobId(artifact.payload.cron_job_id),
      },
    };
  }
  return {
    ...common,
    kind: artifact.kind,
    payload: {
      ...artifact.payload,
      cron_job_id: parseCronJobId(artifact.payload.cron_job_id),
    },
  };
};

const fromApiResponseMessage = (message: IResponseMessage): IResponseMessage => ({
  ...message,
  msg_id: parseMessageId(message.msg_id),
  turn_id: message.turn_id == null ? undefined : parseMessageId(message.turn_id),
  final_text_msg_id:
    message.final_text_msg_id == null ? undefined : parseMessageId(message.final_text_msg_id),
  conversation_id: parseConversationId(message.conversation_id),
  companion_id:
    message.companion_id == null ? message.companion_id : parseCompanionId(message.companion_id),
});

const fromApiKnowledgeWritebackEvent = (
  event: IKnowledgeWritebackEvent
): IKnowledgeWritebackEvent => ({
  ...event,
  conversation_id: parseConversationId(event.conversation_id),
  msg_id: parseMessageId(event.msg_id),
  written: event.written?.map((item) => ({
    ...item,
    kb_id: item.kb_id == null ? item.kb_id : parseKnowledgeBaseId(item.kb_id),
  })),
  failures: event.failures?.map((item) => ({
    ...item,
    kb_id: item.kb_id == null ? item.kb_id : parseKnowledgeBaseId(item.kb_id),
  })),
});

const fromApiUserMessageCreatedEvent = (
  event: IUserMessageCreatedEvent
): IUserMessageCreatedEvent => ({
  ...event,
  conversation_id: parseConversationId(event.conversation_id),
  msg_id: parseMessageId(event.msg_id),
  companion_id:
    event.companion_id == null ? event.companion_id : parseCompanionId(event.companion_id),
});

export const fromApiTurnCompletedEvent = (raw: unknown): IConversationTurnCompletedEvent => {
  const r = raw as Record<string, unknown>;
  const rawLast = r.last_message as Record<string, unknown> | undefined;
  if (rawLast && Object.prototype.hasOwnProperty.call(rawLast, 'id')) {
    throw new TypeError('turn.completed last_message legacy field "id" is not accepted; use "message_id"');
  }
  const last_message: IConversationTurnCompletedEvent['last_message'] = rawLast
    ? {
        message_id: rawLast.message_id == null ? undefined : parseMessageId(rawLast.message_id),
        type: rawLast.type as string | undefined,
        content: rawLast.content ?? null,
        status: rawLast.status as string | null | undefined,
        created_at: (rawLast.created_at ?? Date.now()) as number,
      }
    : {
        content: null,
        created_at: Date.now(),
      };
  const rawRuntime = (r.runtime ?? {}) as Record<string, unknown>;
  const runtime: IConversationTurnCompletedEvent['runtime'] = {
    state: (rawRuntime.state ?? 'idle') as IConversationTurnCompletedEvent['runtime']['state'],
    can_send_message: (rawRuntime.can_send_message ?? true) as boolean,
    has_runtime: (rawRuntime.has_runtime ?? false) as boolean,
    runtime_status: rawRuntime.runtime_status as IConversationTurnCompletedEvent['runtime']['runtime_status'],
    // Missing terminal runtime authority must never be interpreted as an
    // already-released turn. Lifecycle consumers fail closed on `true`.
    is_processing:
      typeof rawRuntime.is_processing === 'boolean' ? rawRuntime.is_processing : true,
    pending_confirmations: (rawRuntime.pending_confirmations ?? 0) as number,
    ...(rawRuntime.active_turn_id == null
      ? {}
      : { active_turn_id: parseMessageId(rawRuntime.active_turn_id) }),
  };
  const rawModel = (r.model ?? {}) as Record<string, unknown>;
  const model: IConversationTurnCompletedEvent['model'] = {
    platform: (rawModel.platform ?? '') as string,
    name: (rawModel.name ?? '') as string,
    use_model: (rawModel.use_model ?? '') as string,
  };
  return {
    conversation_id: parseConversationId(r.conversation_id),
    turn_id: r.turn_id == null ? undefined : parseMessageId(r.turn_id),
    status: (r.status ?? 'finished') as IConversationTurnCompletedEvent['status'],
    state: (r.state ??
      (r.status === 'finished' ? 'ai_waiting_input' : 'unknown')) as IConversationTurnCompletedEvent['state'],
    detail: (r.detail ?? '') as string,
    can_send_message: (r.can_send_message ?? r.status === 'finished') as boolean,
    runtime,
    workspace: (r.workspace ?? '') as string,
    model,
    last_message,
  };
};

/** In-session companion summon marker persisted at `extra.summon`（设计 B）。 */
export interface ISummonConfig {
  companion_id: CompanionId;
  memory_ids: CompanionMemoryId[];
  skill_exclusions: string[];
  /** Server-stamped epoch ms — clients never set it. */
  summoned_at: number;
}

export interface ISetSummonParams {
  conversation_id: ConversationId;
  companion_id: CompanionId;
  memory_ids?: CompanionMemoryId[];
  skill_exclusions?: string[];
}

export const conversation = {
  create: withResponseMap(
    httpPost<unknown, ICreateConversationParams>('/api/conversations', (p) => {
      // Top-level `model` is nomi-only on the backend (spec 2026-05-12).
      // Other agent types carry model info via `extra`.
      const isNomi = p.type === 'nomi';
      // Conversations are minted by the backend; never send a client-supplied
      // entity ID.
      const body: Record<string, unknown> = {
        type: p.type,
        name: p.name,
        preset_id: p.preset_id,
        preset_overrides: p.preset_overrides,
        extra: p.extra,
      };
      if (isNomi) {
        const model = toApiModelOptional(p.model);
        if (model) body.model = model;
        if (p.delegation_policy) body.delegation_policy = p.delegation_policy;
        if (p.execution_model_pool) body.execution_model_pool = p.execution_model_pool;
        if (p.decision_policy) body.decision_policy = p.decision_policy;
        if (p.execution_template_id) body.execution_template_id = p.execution_template_id;
      }
      return body;
    }),
    fromApiConversation
  ),
  get: withResponseMap(
    httpGet<unknown, { conversation_id: ConversationId }>(
      (p) => `/api/conversations/${p.conversation_id}`,
      { silentStatuses: [404] }
    ),
    fromApiConversation
  ),
  getAssociateConversation: withResponseMap(
    httpGet<unknown[], { conversation_id: ConversationId }>(
      (p) => `/api/conversations/${p.conversation_id}/associated`
    ),
    (list) => list.map(fromApiConversation)
  ),
  listByCronJob: withResponseMap(
    httpGet<unknown[], { cron_job_id: CronJobId }>((p) => `/api/cron/jobs/${p.cron_job_id}/conversations`),
    (list) => list.map(fromApiConversation)
  ),
  remove: httpDelete<void, { conversation_id: ConversationId }>(
    (p) => `/api/conversations/${p.conversation_id}`
  ),
  // updates 额外允许顶层 `pinned`：对应 conversations 表真列（UpdateConversationRequest.pinned，
  // 服务端置位时自动维护 pinned_at）；body 构造的 `...rest` 原样透传该字段。
  // 注意：不要往 body 里加任何 UpdateConversationRequest 之外的字段——该 DTO 是
  // `deny_unknown_fields`，多一个键整条 PATCH 直接 400。`extra` 恒为合并语义
  // （见 nomifun-conversation/src/service.rs 的 update），无需任何开关字段。
  //
  // `extra` 单独放宽为 Partial：它是合并语义，调用方本就只传要改的键，而
  // `Partial<TChatConversation>` 作用在联合类型上时仍要求 `extra` 整体符合某一
  // 分支。此前有一个全可选的分支意外充当了逃逸口，该分支随引擎删除后消失。
  update: httpPatch<
    boolean,
    {
      conversation_id: ConversationId;
      updates: (Partial<TChatConversation> | { extra: Partial<TChatConversation['extra']> }) & {
        pinned?: boolean;
      };
    }
  >(
    (p) => `/api/conversations/${p.conversation_id}`,
    (p) => {
      const updates = p.updates as Record<string, unknown>;
      const { model: rawModel, ...rest } = updates;
      const model = toApiModelOptional(rawModel as TProviderWithModel | undefined);
      return {
        ...rest,
        ...(model ? { model } : {}),
      };
    }
  ),
  reset: httpPost<void, IResetConversationParams>((p) => `/api/conversations/${p.conversation_id}/reset`),
  warmup: httpPost<void, { conversation_id: ConversationId }>((p) => `/api/conversations/${p.conversation_id}/warmup`),
  stop: httpPost<void, { conversation_id: ConversationId }>((p) => `/api/conversations/${p.conversation_id}/cancel`),
  clearContext: httpPost<void, { conversation_id: ConversationId }>(
    (p) => `/api/conversations/${p.conversation_id}/clear-context`
  ),
  /** 清空一条会话的全部消息（保留会话行，不触碰 companion_memories 记忆库）。
   *  伙伴专属会话「清空上下文」按钮调用。 */
  clearMessages: httpPost<boolean, { conversation_id: ConversationId }>(
    (p) => `/api/conversations/${p.conversation_id}/clear-messages`
  ),
  /** 召唤伙伴（设计 B）：把一位伙伴的技能与勾选记忆（只读）装进这条工作会话。
   *  服务端盖 summoned_at 并回收运行时，下一条消息生效；会话非空闲返回 409。 */
  setSummon: httpPut<ISummonConfig, ISetSummonParams>(
    (p) => `/api/conversations/${p.conversation_id}/summon`,
    (p) => ({
      companion_id: p.companion_id,
      memory_ids: p.memory_ids,
      skill_exclusions: p.skill_exclusions,
    })
  ),
  /** 解除召唤（幂等）；非空闲 409。技能目录在下一次运行时构建时按 manifest 卸载。 */
  clearSummon: httpDelete<void, { conversation_id: ConversationId }>(
    (p) => `/api/conversations/${p.conversation_id}/summon`
  ),
  retryKnowledgeWriteback: httpPost<
    void,
    { conversation_id: ConversationId; message_id: MessageId; attempt_id: string }
  >(
    (p) =>
      `/api/conversations/${p.conversation_id}/messages/${p.message_id}/knowledge-writeback/retry`,
    (p) => ({ attempt_id: p.attempt_id })
  ),
  activeCount: httpGet<{ count: number }>('/api/conversations/active-count'),
  sendMessage: {
    provider: () => {},
    invoke: async (p: ISendMessageParams): Promise<ISendMessageResult> => {
      const idempotencyKey = requireConversationIdempotencyKey(p.idempotency_key);
      const result = await httpRequest<ISendMessageResult>(
        'POST',
        `/api/conversations/${p.conversation_id}/messages`,
        {
          content: p.input,
          files: p.files,
          inject_skills: p.inject_skills,
        },
        { idempotencyKey, initialOnly: p.initial_only === true }
      );
      return fromApiSendMessageResult(result);
    },
  },
  steer: {
    provider: () => {},
    invoke: async (p: ISendMessageParams): Promise<ISendMessageResult> => {
      const idempotencyKey = requireConversationIdempotencyKey(p.idempotency_key);
      const result = await httpRequest<ISendMessageResult>(
        'POST',
        `/api/conversations/${p.conversation_id}/steer`,
        {
        content: p.input,
        files: p.files,
        inject_skills: p.inject_skills,
        },
        { idempotencyKey }
      );
      return fromApiSendMessageResult(result);
    },
  },
  editResubmit: {
    provider: () => {},
    invoke: async (p: {
      conversation_id: ConversationId;
      msg_id: MessageId;
      input: string;
      files?: string[];
      idempotency_key: string;
    }): Promise<ISendMessageResult> => {
      const idempotencyKey = requireConversationIdempotencyKey(p.idempotency_key);
      const result = await httpRequest<ISendMessageResult>(
        'POST',
        `/api/conversations/${p.conversation_id}/messages/${p.msg_id}/edit-resubmit`,
        {
        content: p.input,
        files: p.files,
        },
        { idempotencyKey }
      );
      return fromApiSendMessageResult(result);
    },
  },
  continueTruncated: {
    provider: () => {},
    invoke: async (p: {
      conversation_id: ConversationId;
      source_message_id: MessageId;
      idempotency_key: string;
    }): Promise<ISendMessageResult> => {
      const idempotencyKey = requireConversationIdempotencyKey(p.idempotency_key);
      const result = await httpRequest<ISendMessageResult>(
        'POST',
        `/api/conversations/${p.conversation_id}/messages/${p.source_message_id}/continue-truncated`,
        undefined,
        { idempotencyKey }
      );
      return fromApiSendMessageResult(result);
    },
  },
  getSlashCommands: httpGet<Array<{ command: string; description: string }>, { conversation_id: ConversationId }>(
    (p) => `/api/conversations/${p.conversation_id}/slash-commands`
  ),
  askSideQuestion: httpPost<ConversationSideQuestionResult, { conversation_id: ConversationId; question: string }>(
    (p) => `/api/conversations/${p.conversation_id}/side-question`,
    (p) => ({ question: p.question })
  ),
  confirmMessage: httpPost<void, IConfirmMessageParams>(
    (p) => `/api/conversations/${p.conversation_id}/confirmations/${encodeURIComponent(p.call_id)}/confirm`,
    (p) => ({ msg_id: p.msg_id, data: p.confirm_key })
  ),
  listArtifacts: withResponseMap(
    httpGet<ConversationArtifactResponse[], { conversation_id: ConversationId }>(
      (p) => `/api/conversations/${p.conversation_id}/artifacts`
    ),
    (artifacts) => artifacts.map(fromApiConversationArtifact)
  ),
  updateArtifact: withResponseMap(
    httpPatch<
      ConversationArtifactResponse,
      {
        conversation_id: ConversationId;
        conversation_artifact_id: ConversationArtifactId;
        status: IConversationArtifactStatus;
      }
    >(
      (p) =>
        `/api/conversations/${p.conversation_id}/artifacts/${p.conversation_artifact_id}`,
      (p) => ({ status: p.status })
    ),
    fromApiConversationArtifact
  ),
  responseStream: wsMappedEmitter<IResponseMessage>('message.stream', (raw) =>
    fromApiResponseMessage(raw as IResponseMessage)
  ),
  /** A user message was persisted (incl. IM channel inbound — see
   *  IUserMessageCreatedEvent). */
  userCreated: wsMappedEmitter<IUserMessageCreatedEvent>('message.userCreated', (raw) =>
    fromApiUserMessageCreatedEvent(raw as IUserMessageCreatedEvent)
  ),
  artifactStream: wsMappedEmitter<IConversationArtifact, ConversationArtifactResponse>(
    'conversation.artifact',
    fromApiConversationArtifact
  ),
  knowledgeWriteback: wsMappedEmitter<IKnowledgeWritebackEvent>('knowledge.writeback', (raw) =>
    fromApiKnowledgeWritebackEvent(raw as IKnowledgeWritebackEvent)
  ),
  /** The server does not replay WebSocket frames. Consumers with durable
   * projections must reload them after a successful reconnect. */
  reconnected: wsEmitter<undefined>('ws.reconnected'),
  turnStarted: wsMappedEmitter<IConversationTurnStartedEvent, unknown>('turn.started', (raw) => {
    const r = raw as Record<string, unknown>;
    const rawRuntime = (r.runtime ?? {}) as Record<string, unknown>;
    const rawProcessingStartedAt = rawRuntime.processing_started_at;
    const processing_started_at =
      typeof rawProcessingStartedAt === 'number'
        ? rawProcessingStartedAt
        : typeof rawProcessingStartedAt === 'string'
          ? Number(rawProcessingStartedAt)
          : undefined;
    return {
      conversation_id: parseConversationId(r.conversation_id),
      turn_id: parseMessageId(r.turn_id),
      status: (r.status ?? 'running') as IConversationTurnStartedEvent['status'],
      phase: (r.phase ?? 'starting') as IConversationTurnStartedEvent['phase'],
      state: (r.state ?? 'initializing') as IConversationTurnStartedEvent['state'],
      detail: (r.detail ?? '') as string,
      can_send_message: (r.can_send_message ?? false) as boolean,
      runtime: {
        state: (rawRuntime.state ?? 'starting') as IConversationTurnStartedEvent['runtime']['state'],
        can_send_message: (rawRuntime.can_send_message ?? false) as boolean,
        has_runtime: (rawRuntime.has_runtime ?? true) as boolean,
        runtime_status: rawRuntime.runtime_status as IConversationTurnStartedEvent['runtime']['runtime_status'],
        is_processing: (rawRuntime.is_processing ?? true) as boolean,
        pending_confirmations: (rawRuntime.pending_confirmations ?? 0) as number,
        ...(rawRuntime.active_turn_id == null
          ? {}
          : { active_turn_id: parseMessageId(rawRuntime.active_turn_id) }),
        ...(Number.isFinite(processing_started_at) ? { processing_started_at } : {}),
      },
      companion: r.companion as boolean | undefined,
      companion_id: r.companion_id == null ? null : parseCompanionId(r.companion_id),
      origin: (r.origin ?? null) as string | null | undefined,
      channel_platform: r.channel_platform as string | null | undefined,
    };
  }),
  turnCompleted: wsMappedEmitter<IConversationTurnCompletedEvent, unknown>('turn.completed', fromApiTurnCompletedEvent),
  listChanged: wsEmitter<IConversationListChangedEvent>('conversation.listChanged'),
  // Uses httpRequest directly (instead of httpGet + withResponseMap) because the
  // response mapper needs `workspace` from params to build fullPath/relativePath,
  // and withResponseMap's map function does not receive the original params.
  getWorkspace: {
    provider: () => {},
    invoke: (async (p: { conversation_id: ConversationId; workspace: string; path: string; search?: string }) => {
      const rel = absoluteToRelativePath(p.path, p.workspace);
      const url = `/api/conversations/${p.conversation_id}/workspace?path=${encodeURIComponent(rel)}${p.search ? `&search=${encodeURIComponent(p.search)}` : ''}`;
      const raw = await httpRequest<Array<{ name: string; type: string }>>('GET', url);
      return fromBackendWorkspaceList(raw, p.workspace, rel);
    }) as (p: { conversation_id: ConversationId; workspace: string; path: string; search?: string }) => Promise<IDirOrFile[]>,
  },
  responseSearchWorkSpace: stubProvider<void, { file: number; dir: number; match?: IDirOrFile }>(
    'responseSearchWorkSpace',
    undefined as unknown as void
  ),
  confirmation: {
    add: wsEmitter<IConfirmation<unknown> & { conversation_id: ConversationId }>('confirmation.add'),
    update: wsEmitter<IConfirmation<unknown> & { conversation_id: ConversationId }>('confirmation.update'),
    confirm: httpPost<
      void,
      {
        conversation_id: ConversationId;
        msg_id: MessageId | ConfirmationCorrelationId;
        data: unknown;
        call_id: string;
        always_allow?: boolean;
      }
    >(
      (p) => `/api/conversations/${p.conversation_id}/confirmations/${encodeURIComponent(p.call_id)}/confirm`,
      (p) => ({
        msg_id: p.msg_id,
        data: p.data,
        always_allow: p.always_allow ?? false,
      })
    ),
    list: httpGet<IConfirmation<unknown>[], { conversation_id: ConversationId }>(
      (p) => `/api/conversations/${p.conversation_id}/confirmations`
    ),
    remove: wsEmitter<{ conversation_id: ConversationId; id: string }>('confirmation.remove'),
  },
  approval: {
    check: httpGet<{ approved: boolean }, { conversation_id: ConversationId; action: string; command_type?: string }>(
      (p) =>
        `/api/conversations/${p.conversation_id}/approvals/check?action=${encodeURIComponent(p.action)}${p.command_type ? `&command_type=${encodeURIComponent(p.command_type)}` : ''}`
    ),
  },
};

export interface IStartOnBootStatus {
  supported: boolean;
  enabled: boolean;
  isPackaged: boolean;
  platform: string;
}

export type IRendererLogLevel = 'info' | 'warn' | 'error';

export interface IRendererLogEntry {
  level: IRendererLogLevel;
  tag: string;
  message: string;
  data?: unknown;
}

// ---------------------------------------------------------------------------
// Application — stays IPC (Electron-native)
// ---------------------------------------------------------------------------

export const application = {
  restart: shellProvider<void, void>(() => tauriRelaunch(), undefined),
  // Arm a factory reset: the backend writes a marker and the wipe happens early
  // on the next boot (see nomifun_common::factory_reset). Callers should relaunch
  // (application.restart) right after this resolves.
  factoryReset: httpPost<void, void>('/api/system/factory-reset'),
  // DEGRADE_STUB: Tauri v2 has no public JS API to toggle the webview devtools.
  openDevTools: stubShellProvider<boolean, void>(false),
  systemInfo: withResponseMap(
    httpGet<
      {
        cache_dir: string;
        work_dir: string;
        log_dir: string;
        storage_generation: string;
        platform: string;
        arch: string;
      },
      void
    >('/api/system/info'),
    (raw) => ({
      cacheDir: raw.cache_dir,
      workDir: raw.work_dir,
      logDir: raw.log_dir,
      storageGeneration: raw.storage_generation,
      platform: raw.platform,
      arch: raw.arch,
    })
  ),
  getPath: shellProvider<string, { name: 'desktop' | 'home' | 'downloads' }>(({ name }) => tauriGetPath(name), ''),
  // Persist the user-chosen work dir to a pre-boot config file that the next
  // boot reads before resolving work_dir (Rust `nomifun_common::dir_config`).
  // The caller restarts right after this resolves; the new dir applies then.
  // `cacheDir` is accepted for back-compat but ignored — it is no longer
  // user-editable (removed from the settings UI), only `workDir` is sent.
  updateSystemInfo: httpPost<void, { cacheDir: string; workDir: string }>(
    '/api/system/work-dir',
    ({ workDir }) => ({ work_dir: workDir })
  ),
  getZoomFactor: shellProvider<number, void>(async () => tauriGetZoom(), 1),
  setZoomFactor: shellProvider<number, { factor: number }>(({ factor }) => tauriSetZoom(factor), 1),
  applyKeepAwake: shellProvider<void, { enabled: boolean }>(({ enabled }) => tauriSetKeepAwake(enabled), undefined),
  // Localize the native system-tray menu. Desktop-only OS effect (no-op on web),
  // mirroring applyKeepAwake — the renderer calls it on mount / language change.
  setTrayLabels: shellProvider<void, { show: string; quit: string }>(
    ({ show, quit }) => tauriSetTrayLabels(show, quit),
    undefined
  ),
  getStartOnBootStatus: shellProvider<IBridgeResponse<IStartOnBootStatus>, void>(
    async () => ({
      success: true,
      data: { supported: true, enabled: await tauriIsAutostartEnabled(), isPackaged: true, platform: navigator.platform },
    }),
    { success: false }
  ),
  setStartOnBoot: shellProvider<IBridgeResponse<IStartOnBootStatus>, { enabled: boolean }>(
    async ({ enabled }) => {
      await tauriSetAutostart(enabled);
      return {
        success: true,
        data: {
          supported: true,
          enabled,
          isPackaged: true,
          platform: navigator.platform,
        },
      };
    },
    { success: false }
  ),
  // DEGRADE_STUB: renderer-log piping to the shell; the in-process backend owns log files.
  writeRendererLog: stubShellProvider<void, IRendererLogEntry>(undefined),
  logStream: noopEmitter<{
    level: 'log' | 'warn' | 'error';
    tag: string;
    message: string;
    data?: unknown;
  }>(),
};

// ---------------------------------------------------------------------------
// Update — stays IPC (Electron-native auto-updater)
// ---------------------------------------------------------------------------

// Tauri-native auto-update, backed by @tauri-apps/plugin-updater (see
// ./tauriUpdater). The in-app UpdateModal drives this flow: it calls
// `autoUpdate.check` then `update.check`, and — because the Tauri updater plugin
// downloads + installs internally (no per-asset manual download, so
// `recommendedAsset` is intentionally absent) — routes the download through
// `autoUpdate.download`. The modal is shell-gated (About entry + startup check
// only render under `isDesktopShell()`), and `shellProvider` additionally guards
// each call with `isTauriRuntime()`, so the WebUI browser degrades to the safe fallback.

/** Releases page shown in the modal's "go to release" affordance. */
const GITHUB_RELEASES_PAGE = 'https://github.com/nomifun/nomifun-tauri/releases/latest';

export const update = {
  open: noopEmitter<{ source?: 'menu' | 'about' }>(),
  check: shellProvider<IBridgeResponse<UpdateCheckResult>, UpdateCheckRequest>(async () => {
    // Reuses the check started by autoUpdate.check (the modal calls that first),
    // so this is the SAME round-trip, not a second network call.
    const currentVersion = await tauriUpdateCurrentVersion();
    const info = await tauriUpdateCheck(false);
    if (!info) {
      return { success: true, data: { currentVersion, updateAvailable: false } };
    }
    const latest: UpdateReleaseInfo = {
      tagName: `v${info.version}`,
      version: info.version,
      body: info.releaseNotes,
      htmlUrl: GITHUB_RELEASES_PAGE,
      prerelease: false,
      draft: false,
      assets: [],
      // recommendedAsset intentionally omitted: the plugin handles download +
      // install, so the modal routes through the autoUpdate.* channels below.
    };
    return { success: true, data: { currentVersion, updateAvailable: true, latest } };
  }, { success: false, msg: 'Updater is unavailable outside the desktop shell' }),
  // Unused under Tauri (no recommendedAsset → the modal never takes the manual
  // download path); kept for API compatibility with the modal's manual branch.
  download: stubShellProvider<IBridgeResponse<UpdateDownloadResult>, UpdateDownloadRequest>({
    success: false,
    msg: 'Use the Tauri updater (auto path)',
  }),
  downloadProgress: noopEmitter<UpdateDownloadProgressEvent>(),
};

export const autoUpdate = {
  check: shellProvider<
    IBridgeResponse<{
      updateInfo?: {
        version: string;
        releaseDate?: string;
        releaseNotes?: string;
      };
      /**
       * Version whose verified package the native side already holds, if any.
       * The modal derives its install affordance from this instead of trusting
       * React state to have survived a re-check.
       */
      retainedVersion?: string | null;
      /** Native slot state, so a re-check can also land on "already downloading". */
      packageState?: import('./tauriShell').TauriUpdatePackageState | null;
      packageVersion?: string | null;
    }>,
    { includePrerelease?: boolean }
  >(async () => {
    // `force` so each modal open / retry performs a fresh check; update.check
    // (called right after) then reuses this same in-flight result.
    const info = await tauriUpdateCheck(true);
    const snapshot = await tauriUpdatePackageSnapshot();
    const slot = {
      retainedVersion: snapshot?.state === 'ready' ? (snapshot.version ?? null) : null,
      packageState: snapshot?.state ?? null,
      packageVersion: snapshot?.version ?? null,
    };
    if (!info) return { success: true, data: slot };
    return {
      success: true,
      data: {
        updateInfo: { version: info.version, releaseDate: info.releaseDate, releaseNotes: info.releaseNotes },
        ...slot,
      },
    };
  }, { success: false }),
  download: shellProvider<IBridgeResponse, void>(async () => {
    await tauriUpdateDownload((s) => autoUpdateStatusEmitter.emit(s));
    return { success: true };
  }, { success: false }),
  quitAndInstall: shellProvider<void, void>(
    () => tauriUpdateInstallAndRelaunch((s) => autoUpdateStatusEmitter.emit(s)),
    undefined
  ),
  status: autoUpdateStatusEmitter,
};

// ---------------------------------------------------------------------------
// Star Office — routed to backend
// ---------------------------------------------------------------------------

export const starOffice = {
  detectUrl: httpPost<{ url: string | null }, { preferredUrl?: string; force?: boolean; timeoutMs?: number }>(
    '/api/star-office/detect'
  ),
};

// ---------------------------------------------------------------------------
// Dialog — stays IPC (native file picker)
// ---------------------------------------------------------------------------

export const dialog = {
  showOpen: shellProvider<string[] | undefined, ShellOpenDialogOptions | void>(
    (opts) => tauriOpenDialog(opts || undefined),
    (opts) => bridge.invoke<string[] | undefined>('show-open', opts || undefined)
  ),
};

// ---------------------------------------------------------------------------
// File System — routed to /api/fs/* and /api/skills/*
// ---------------------------------------------------------------------------

export type SkillMarketSource =
  | 'clawhub'
  | 'skillhub'
  | 'loophub'
  | 'skillhub_mcp'
  | 'mcpworld'
  | 'clawhub_plugins';

export interface ISkillMarketItem {
  id: string;
  source: SkillMarketSource;
  rank: number;
  name: string;
  description: string;
  url: string;
  install_command: string;
  tags?: string[];
  audience_tags?: string[];
  scenario_tags?: string[];
  stats?: string;
}

export interface ISkillMarketSyncResponse {
  fetched_at: number;
  items: ISkillMarketItem[];
  errors?: string[];
}

export interface ISkillMarketMcpConfigResponse {
  config_json: unknown;
}

export const fs = {
  listWorkspaceFiles: withResponseMap(
    httpPost<Array<RawWorkspaceFlatFile>, { root: string }>('/api/fs/list'),
    fromBackendWorkspaceFlatFiles
  ),
  getImageBase64: httpPost<string | null, { path: string; workspace?: string }>('/api/fs/image-base64'),
  fetchRemoteImage: httpPost<string, { url: string }>('/api/fs/fetch-remote-image'),
  readFile: httpPost<string | null, { path: string; workspace?: string }>('/api/fs/read'),
  writeFile: httpPost<boolean, { path: string; data: string }>('/api/fs/write'),
  createZip: httpPost<
    boolean,
    {
      path: string;
      request_id?: string;
      files: Array<{
        name: string;
        content?: string | Uint8Array;
        source_path?: string;
      }>;
    }
  >('/api/fs/zip'),
  cancelZip: httpPost<boolean, { request_id: string }>('/api/fs/zip/cancel'),
  getFileMetadata: httpPost<IFileMetadata, { path: string; workspace?: string }>('/api/fs/metadata'),
  copyFilesToWorkspace: httpPost<
    {
      copied_files: string[];
      failed_files?: Array<{ path: string; error: string }>;
    },
    { file_paths: string[]; workspace: string; source_root?: string }
  >('/api/fs/copy'),
  removeEntry: httpPost<void, { path: string }>('/api/fs/remove'),
  renameEntry: httpPost<{ new_path: string }, { path: string; new_name: string }>('/api/fs/rename'),
  readBuiltinRule: httpPost<string, { file_name: string }>('/api/skills/builtin-rule'),
  readBuiltinSkill: httpPost<string, { file_name: string }>('/api/skills/builtin-skill'),
  listAvailableSkills: httpGet<
    Array<{
      name: string;
      description: string;
      name_i18n?: Record<string, string>;
      description_i18n?: Record<string, string>;
      location: string;
      relative_location?: string;
      is_custom: boolean;
      source: 'builtin' | 'custom' | 'extension';
      audience_tags?: string[];
      scenario_tags?: string[];
    }>,
    void
  >('/api/skills'),
  listBuiltinAutoSkills: httpGet<
    Array<{ name: string; description: string; name_i18n?: Record<string, string>; description_i18n?: Record<string, string>; location: string }>,
    void
  >('/api/skills/builtin-auto'),
  materializeSkillsForAgent: httpPost<
    { skills: Array<{ name: string; source_path: string }> },
    { conversation_id: ConversationId; skills: string[] }
  >('/api/skills/materialize-for-agent'),
  readSkillInfo: httpPost<{ name: string; description: string }, { skill_path: string }>('/api/skills/info'),
  importSkill: httpPost<{ skill_name: string }, { skill_path: string }>('/api/skills/import'),
  scanForSkills: httpPost<Array<{ name: string; description: string; path: string }>, { folder_path: string }>(
    '/api/skills/scan'
  ),
  detectCommonSkillPaths: httpGet<Array<{ name: string; path: string }>, void>('/api/skills/detect-paths'),
  detectAndCountExternalSkills: httpGet<
    Array<{
      name: string;
      path: string;
      source: string;
      skill_count: number;
      skills: Array<{ name: string; description: string; path: string }>;
    }>,
    void
  >('/api/skills/detect-external'),
  importSkillWithSymlink: httpPost<{ skill_name: string; skill_names?: string[] }, { skill_path: string }>(
    '/api/skills/import-symlink'
  ),
  deleteSkill: httpDelete<void, { skill_name: string }>((p) => `/api/skills/${encodeURIComponent(p.skill_name)}`),
  // Assign tags to a skill (PUT /api/skills/{name}/tags). Tag keys reference the
  // shared preset tag vocabulary; the backend stores them in a sidecar table.
  setSkillTags: httpPut<void, { skill_name: string; audience_tags: string[]; scenario_tags: string[] }>(
    (p) => `/api/skills/${encodeURIComponent(p.skill_name)}/tags`,
    (p) => ({ audience_tags: p.audience_tags, scenario_tags: p.scenario_tags })
  ),
  getSkillPaths: httpGet<{ user_skills_dir: string; builtin_skills_dir: string }, void>('/api/skills/paths'),
  getCustomExternalPaths: httpGet<Array<{ name: string; path: string }>, void>('/api/skills/external-paths'),
  addCustomExternalPath: httpPost<void, { name: string; path: string }>('/api/skills/external-paths'),
  removeCustomExternalPath: httpDelete<void, { path: string }>(
    (p) => `/api/skills/external-paths?path=${encodeURIComponent(p.path)}`
  ),
  enableSkillsMarket: httpPost<void, void>('/api/skills/market/enable'),
  disableSkillsMarket: httpPost<void, void>('/api/skills/market/disable'),
  syncSkillMarketRankings: httpPost<ISkillMarketSyncResponse, { sources?: SkillMarketSource[] }>(
    '/api/skills/market/rankings/sync'
  ),
  resolveSkillMarketMcpConfig: httpPost<
    ISkillMarketMcpConfigResponse,
    { source: SkillMarketSource; id: string; url: string }
  >('/api/skills/market/mcp/config'),
};

// Workspace Office file watch
export const workspaceOfficeWatch = {
  start: httpPost<void, { workspace: string }>('/api/fs/office-watch/start'),
  stop: httpPost<void, { workspace: string }>('/api/fs/office-watch/stop'),
  fileAdded: wsEmitter<{ file_path: string; workspace: string }>('workspaceOfficeWatch.fileAdded'),
};

// File streaming updates (real-time content push when agent writes)
export const fileStream = {
  contentUpdate: wsEmitter<{
    file_path: string;
    content: string;
    workspace: string;
    relative_path: string;
    operation: 'write' | 'delete';
  }>('fileStream.contentUpdate'),
};

// File snapshot providers
export const fileSnapshot = {
  init: httpPost<import('@/common/types/platform/fileSnapshot').SnapshotInfo, { workspace: string }>(
    '/api/fs/snapshot/init'
  ),
  compare: withResponseMap(
    httpPost<RawCompareResult, { workspace: string }>('/api/fs/snapshot/compare'),
    fromBackendCompareResult
  ),
  getBaselineContent: httpPost<string | null, { workspace: string; file_path: string }>('/api/fs/snapshot/baseline'),
  dispose: httpPost<void, { workspace: string }>('/api/fs/snapshot/dispose'),
  stageFile: httpPost<void, { workspace: string; file_path: string }>('/api/fs/snapshot/stage'),
  stageAll: httpPost<void, { workspace: string }>('/api/fs/snapshot/stage-all'),
  unstageFile: httpPost<void, { workspace: string; file_path: string }>('/api/fs/snapshot/unstage'),
  unstageAll: httpPost<void, { workspace: string }>('/api/fs/snapshot/unstage-all'),
  discardFile: httpPost<
    void,
    {
      workspace: string;
      file_path: string;
      operation: import('@/common/types/platform/fileSnapshot').FileChangeOperation;
    }
  >('/api/fs/snapshot/discard'),
  resetFile: httpPost<
    void,
    {
      workspace: string;
      file_path: string;
      operation: import('@/common/types/platform/fileSnapshot').FileChangeOperation;
    }
  >('/api/fs/snapshot/reset'),
};

// ---------------------------------------------------------------------------
// Mode (Provider management) — routed to /api/providers/*
// ---------------------------------------------------------------------------

const normalizeManagedModelStatus = (
  status: ManagedModelServiceStatus
): ManagedModelServiceStatus => ({
  ...status,
  providerId: status.providerId == null ? null : parseProviderId(status.providerId),
});

export const mode = {
  listProviders: withResponseMap(httpGet<ProviderResponse[], void>('/api/providers'), (providers) =>
    providers.map(fromProviderResponse)
  ),
  createProvider: withResponseMap(
    httpPost<ProviderResponse, CreateProviderInput>('/api/providers', toCreateProviderRequest),
    fromProviderResponse
  ),
  updateProvider: withResponseMap(httpPut<ProviderResponse, { provider_id: ProviderId } & UpdateProviderRequest>(
    (p) => `/api/providers/${p.provider_id}`,
    // Call sites may derive this object from a whole renderer record or form.
    // Serialize only the strict UpdateProviderRequest contract: response-only
    // (nested models) and form-only fields must not
    // reach the backend's deny_unknown_fields DTO.
    toUpdateProviderRequest
  ), fromProviderResponse),
  /** Read the provider's saved API keys in plaintext for the model-management editor. */
  getProviderApiKeys: httpGet<string[], { provider_id: ProviderId }>(
    (p) => `/api/providers/${p.provider_id}/api-keys`
  ),
  deleteProvider: httpDelete<void, { provider_id: ProviderId }>(
    (p) => `/api/providers/${p.provider_id}`
  ),
  /**
   * Server-side provider clone (`POST /api/providers/{id}/clone`): copies the
   * provider row plus every `provider_models` profile row (minus per-deployment
   * health) and every connection profile. Optional body `{ name }` sets the
   * copy's display name (e.g. a localized "<source> 副本"); omitted → the
   * backend picks its default copy name.
   */
  cloneProvider: withResponseMap(
    httpPost<ProviderResponse, { provider_id: ProviderId; name?: string }>(
      (p) => `/api/providers/${p.provider_id}/clone`,
      (p) => (p.name === undefined ? undefined : { name: p.name })
    ),
    fromProviderResponse
  ),
  fetchProviderModels: httpPost<FetchModelsResponse, { provider_id: ProviderId; try_fix?: boolean }>(
    (p) => `/api/providers/${p.provider_id}/models`,
    (p) => ({ try_fix: p.try_fix })
  ),
  /**
   * Pre-create form preview — anonymous fetch-models (T1b).
   * Takes credentials in the body, no provider row required. Used by
   * AddPlatformModal / EditModeModal while the dropdown is still being
   * populated.
   */
  fetchModelList: httpPost<FetchModelsResponse, FetchModelsAnonymousRequest>('/api/providers/fetch-models'),
  /**
   * Reachability test for a saved provider's connection root. Needs no model or
   * capability row, so it can answer before anything is configured on top.
   */
  probeProviderConnection: httpPost<
    ProbeProviderConnectionResponse,
    { provider_id: ProviderId } & ProbeProviderConnectionRequest
  >(
    (p) => `/api/providers/${p.provider_id}/probe-connection`,
    (p) => ({
      protocol: p.protocol,
      task: p.task,
      model: p.model,
      probe_candidates: p.probe_candidates,
    })
  ),
  /** The same test for a proposed connection, before the provider is saved. */
  probeConnection: httpPost<ProbeProviderConnectionResponse, ProbeProviderConnectionAnonymousRequest>(
    '/api/providers/probe-connection'
  ),
};

// ---------------------------------------------------------------------------
// NomiFun-managed free-model service
// ---------------------------------------------------------------------------

export const managedModelService = {
  free: {
    status: withResponseMap(
      httpGet<ManagedModelServiceStatus, void>('/api/model-services/free/status'),
      normalizeManagedModelStatus
    ),
    models: httpGet<ManagedModel[], void>('/api/model-services/free/models'),
    refresh: withResponseMap(
      httpPost<ManagedModelServiceStatus, void>('/api/model-services/free/refresh'),
      normalizeManagedModelStatus
    ),
    setEnabled: withResponseMap(
      httpPost<ManagedModelServiceStatus, SetManagedModelServiceEnabledRequest>(
        '/api/model-services/free/activate'
      ),
      normalizeManagedModelStatus
    ),
    setModelEnabled: withResponseMap(
      httpPatch<ManagedModelServiceStatus, SetManagedModelEnabledRequest>(
        (p) => `/api/model-services/free/models/${encodeURIComponent(p.model_id)}`,
        (p) => ({ enabled: p.enabled })
      ),
      normalizeManagedModelStatus
    ),
    healthSnapshot: httpGet<ManagedModelHealthResult[], void>('/api/model-services/free/health'),
    checkHealth: httpPost<ManagedModelHealthBatchResult, void>('/api/model-services/free/health'),
    checkModelHealth: httpPost<ManagedModelHealthResult, CheckManagedModelHealthRequest>(
      (p) => `/api/model-services/free/models/${encodeURIComponent(p.model_id)}/health`,
      () => undefined
    ),
  },
};

// ---------------------------------------------------------------------------
// Model protocol capability manifest — server-owned operational defaults
// ---------------------------------------------------------------------------

export const modelProtocol = {
  list: httpGet<ModelProtocolManifestResponse, ModelProtocolManifestRequest>(
    (p) => {
      const query = new URLSearchParams({ preset: p.preset, task: p.task });
      if (p.base_url) query.set('base_url', p.base_url);
      if (p.model) query.set('model', p.model);
      return `/api/model-protocols?${query.toString()}`;
    }
  ),
};

// ---------------------------------------------------------------------------
// Provider model catalog (row-level) — routed to /api/provider-models/*
// ---------------------------------------------------------------------------

const normalizeProviderModel = (row: ProviderModelResponse): ProviderModelResponse => ({
  ...row,
  provider_id: parseProviderId(row.provider_id),
});

export const providerModel = {
  /** List catalog rows; pass `provider_id` to filter to one provider. */
  list: withResponseMap(
    httpGet<ProviderModelResponse[], { provider_id?: ProviderId }>((p) =>
      p.provider_id === undefined
        ? '/api/provider-models'
        : `/api/provider-models?provider_id=${encodeURIComponent(p.provider_id)}`
    ),
    (rows) => rows.map(normalizeProviderModel)
  ),
  /** Full upsert: one request replaces the model's complete capability set. */
  save: withResponseMap(
    httpPut<ProviderModelResponse, SaveProviderModelRequest>('/api/provider-models'),
    normalizeProviderModel
  ),
  remove: httpDelete<void, ProviderModelKeyRequest>(
    ({ provider_id, model }) =>
      `/api/provider-models?provider_id=${encodeURIComponent(provider_id)}&model=${encodeURIComponent(model)}`
  ),
};

// ---------------------------------------------------------------------------
// Provider connection profiles (per-role, non-default) —
// routed to /api/providers/{id}/connections[/{role}]
// ---------------------------------------------------------------------------

const normalizeProviderConnection = (
  connection: ProviderConnectionResponse
): ProviderConnectionResponse => ({
  ...connection,
  provider_id: parseProviderId(connection.provider_id),
});

export const providerConnection = {
  list: withResponseMap(
    httpGet<ProviderConnectionResponse[], { provider_id: ProviderId }>(
      (p) => `/api/providers/${p.provider_id}/connections`
    ),
    (connections) => connections.map(normalizeProviderConnection)
  ),
  save: withResponseMap(
    httpPut<
      ProviderConnectionResponse,
      { provider_id: ProviderId; connection: SaveProviderConnectionRequest }
    >(
      (p) => `/api/providers/${p.provider_id}/connections`,
      (p) => p.connection
    ),
    normalizeProviderConnection
  ),
  remove: httpDelete<void, { provider_id: ProviderId; role: string }>(
    (p) => `/api/providers/${p.provider_id}/connections/${encodeURIComponent(p.role)}`
  ),
};

// ---------------------------------------------------------------------------
// Agent Conversation — routed to /api/agents/* + conversation routes
// ---------------------------------------------------------------------------

export const agentConversation = {
  sendMessage: conversation.sendMessage,
  responseStream: conversation.responseStream,
  getAvailableAgents: withResponseMap(
    httpGet<AgentMetadata[], void>('/api/agents'),
    (agents) => agents.map(fromApiAgentMetadata)
  ),
  refreshCustomAgents: httpPost<void, void>('/api/agents/refresh'),
  checkProviderHealth: withResponseMap(
    httpPost<ProviderHealthCheckResponse, ProviderHealthCheckRequest>(
      '/api/agents/provider-health-check'
    ),
    (response) => ({ ...response, provider_id: parseProviderId(response.provider_id) })
  ),
  setMode: httpPut<void, { conversation_id: ConversationId; mode: string }>(
    (p) => `/api/conversations/${p.conversation_id}/mode`,
    (p) => ({ mode: p.mode })
  ),
  // 404 is the expected pre-warmup response from `/api/conversations/:id/mode`
  // — the agent has not attached yet, so we have nothing to read.
  // AgentModeSelector falls back to handshake metadata in that case. Silence
  // the bridge log so this ordinary state doesn't pollute Sentry breadcrumbs
  // (ELECTRON-1BT).
  getMode: httpGet<{ mode: string; initialized: boolean }, { conversation_id: ConversationId }>(
    (p) => `/api/conversations/${p.conversation_id}/mode`,
    {
      silentStatuses: [404],
    }
  ),
};

/**
 * Decode the v3 agent catalog contract at the HTTP boundary.
 *
 * Every `agent_id` is a bare UUIDv7. Catalog lineage belongs in source
 * metadata; the removed ambiguous top-level `id` is never accepted.
 */
function fromApiAgentMetadata(raw: AgentMetadata): AgentMetadata {
  const value = raw as AgentMetadata & Record<string, unknown>;
  if (Object.prototype.hasOwnProperty.call(value, 'id')) {
    throw new TypeError('AgentMetadata legacy field "id" is not accepted; use "agent_id"');
  }
  return { ...raw, agent_id: parseAgentId(value.agent_id) };
}

// ---------------------------------------------------------------------------
// MCP Service — routed to /api/mcp/*
// ---------------------------------------------------------------------------

type ApiMcpServer = Omit<IMcpServer, 'mcp_server_id' | 'original_json'> & {
  mcp_server_id: unknown;
  original_json?: string | null;
};

const fromApiMcpServer = (raw: ApiMcpServer): IMcpServer => {
  if (Object.prototype.hasOwnProperty.call(raw, 'id')) {
    throw new TypeError('MCP server legacy field "id" is not accepted; use "mcp_server_id"');
  }
  return {
    ...raw,
    mcp_server_id: parseMcpServerId(raw.mcp_server_id),
    original_json: raw.original_json ?? '',
  };
};

export type DetectedMcpServer = {
  name: string;
  description?: string;
  transport: IMcpServer['transport'];
  original_json?: string;
  importable: boolean;
  import_skip_reason?: string;
};

export const mcpService = {
  listServers: withResponseMap(
    httpGet<ApiMcpServer[], void>('/api/mcp/servers'),
    (servers) => servers.map(fromApiMcpServer)
  ),
  createServer: withResponseMap(
    httpPost<
      ApiMcpServer,
      Pick<IMcpServer, 'name' | 'description' | 'transport' | 'original_json' | 'builtin'>
    >('/api/mcp/servers'),
    fromApiMcpServer
  ),
  importServers: withResponseMap(
    httpPost<
      ApiMcpServer[],
      {
        servers: Array<Pick<IMcpServer, 'name' | 'description' | 'transport' | 'original_json' | 'builtin'>>;
      }
    >('/api/mcp/servers/import'),
    (servers) => servers.map(fromApiMcpServer)
  ),
  updateServer: withResponseMap(
    httpPut<
      ApiMcpServer,
      {
        mcp_server_id: McpServerId;
        data: Partial<Pick<IMcpServer, 'name' | 'description' | 'transport' | 'original_json' | 'builtin'>>;
      }
    >(
      (p) => `/api/mcp/servers/${p.mcp_server_id}`,
      (p) => p.data
    ),
    fromApiMcpServer
  ),
  deleteServer: httpDelete<void, { mcp_server_id: McpServerId }>(
    (p) => `/api/mcp/servers/${p.mcp_server_id}`
  ),
  toggleServer: withResponseMap(
    httpPost<ApiMcpServer, { mcp_server_id: McpServerId }>(
      (p) => `/api/mcp/servers/${p.mcp_server_id}/toggle`,
      () => undefined
    ),
    fromApiMcpServer
  ),
  getAgentMcpConfigs: httpGet<
    Array<{
      source: string;
      servers: DetectedMcpServer[];
    }>,
    Array<{
      agent_type: string;
      backend?: string;
      name: string;
      cli_path?: string;
    }>
  >('/api/mcp/agent-configs'),
  testMcpConnection: httpPost<
    {
      success: boolean;
      tools?: Array<{
        name: string;
        description?: string;
        input_schema?: unknown;
        _meta?: Record<string, unknown>;
      }>;
      error?: string;
      code?: string;
      details?: unknown;
      needsAuth?: boolean;
      needs_auth?: boolean;
      authMethod?: 'oauth' | 'basic';
      auth_method?: 'oauth' | 'basic';
      wwwAuthenticate?: string;
      www_authenticate?: string;
    },
    McpConnectionTestRequest
  >('/api/mcp/test-connection'),
  checkOAuthStatus: httpPost<{ authenticated: boolean }, { server_url: string }>('/api/mcp/oauth/check-status'),
  loginMcpOAuth: httpPost<{ success: boolean; error?: string }, { server_url: string }>('/api/mcp/oauth/login'),
  logoutMcpOAuth: httpPost<void, { server_url: string }>('/api/mcp/oauth/logout'),
  getAuthenticatedServers: httpGet<string[], void>('/api/mcp/oauth/authenticated'),
};

// ---------------------------------------------------------------------------
// SSH host book — saved, reusable remote-host connection profiles.
// Secrets are write-only from the client: the server returns them masked as
// '***', never as plaintext/ciphertext.
// ---------------------------------------------------------------------------

/** Owner-visible SSH host (secrets masked as '***' when present, else null). */
export interface IApiSshHost {
  sshHostId: SshHostId;
  name: string;
  host: string;
  port: number;
  username: string;
  authType: 'password' | 'key' | 'certificate' | 'agent';
  password: string | null;
  privateKey: string | null;
  passphrase: string | null;
  certificate: string | null;
  sudoPassword: string | null;
  hostFingerprint: string | null;
  status: string;
  lastConnectedAt: number | null;
  createdAt: number;
  updatedAt: number;
}

/** Create payload (secrets are plaintext here, encrypted server-side). */
export interface IApiCreateSshHost {
  name: string;
  host: string;
  port: number;
  username: string;
  authType: 'password' | 'key' | 'certificate' | 'agent';
  password?: string | null;
  privateKey?: string | null;
  passphrase?: string | null;
  certificate?: string | null;
  sudoPassword?: string | null;
}

export type IApiUpdateSshHost = Partial<IApiCreateSshHost>;

/**
 * Live phase of one conversation↔host link (backend `SshLinkPhase`).
 *
 * `degraded` = the transport is fine and the remote shell is being recycled.
 * `dropped` = the link is gone; `detail` says why. `closed` is a finished link,
 * whose `reaped` flag says whether the remote shell was *proven* to have exited.
 */
export type ISshLinkPhase =
  | 'idle'
  | 'connecting'
  | 'connected'
  | 'degraded'
  | 'reconnecting'
  | 'dropped'
  | 'closed';

/**
 * The single wire shape for link state: the realtime `ssh.status` event and the
 * `/api/ssh-hosts/statuses` snapshot both carry it, so a link cannot look
 * different depending on how the client learned about it. Every field is always
 * present — "unknown" is an explicit null, never an omitted key.
 *
 * This — not {@link IApiSshHost.status} — is live link state. The host row's
 * `status` column is a per-host hint that is written on first connect and never
 * walked back, so it is permanently green once a host has ever worked.
 */
export interface IApiSshStatus {
  sshHostId: SshHostId;
  conversationId: string;
  state: ISshLinkPhase;
  /** Which dial attempt this is; 0 outside connecting/reconnecting. */
  attempt: number;
  nextRetryInMs: number | null;
  hostFingerprint: string | null;
  /** Operator-facing transport diagnostics — never credential material. */
  detail: string | null;
  /** Non-null only for `closed`; `false` there means the exit was NOT proven. */
  reaped: boolean | null;
  /**
   * Non-null only for `dropped`. `false` means a retry cannot fix it — a
   * credential was rejected or the host key changed, and a person has to act.
   * Never infer this from `detail`, which is free-form operator text.
   */
  retryable: boolean | null;
  changedAt: number;
}

const fromApiSshHost = (value: IApiSshHost): IApiSshHost => ({
  ...value,
  sshHostId: parseSshHostId(value.sshHostId),
});

/**
 * A host the server found in this machine's `~/.ssh/config` and could add to the
 * host book.
 *
 * Non-secret by construction: `identityFile` is a *path*. The server reads that
 * file's contents only during an import the user confirmed, and never puts key
 * material in this payload.
 */
export interface IApiSshConfigHost {
  /** The `Host` alias, which becomes the host's display name. */
  alias: string;
  /** `HostName`, or the alias itself when the config gives none (ssh semantics). */
  host: string;
  port: number;
  /** `null` when the config names no user — the form asks rather than guesses. */
  username: string | null;
  /** `IdentityFile` with `~/` expanded, or `null`. A path, never contents. */
  identityFile: string | null;
}

/** One read of `~/.ssh/config`, including what it could not offer and why. */
export interface IApiSshConfigScan {
  /** The file that was read, `null` only when this account has no home dir. */
  configPath: string | null;
  hosts: IApiSshConfigHost[];
  /**
   * Aliases left out because they go through a jump host (unsupported in v1).
   * Named rather than dropped: a user whose config is entirely bastion-fronted
   * would otherwise see an empty list with no explanation.
   */
  skippedProxy: string[];
  /**
   * How many `Include` directives the parser did not follow. Reported so a short
   * candidate list is never a silent one.
   */
  skippedIncludes: number;
}

export interface IApiSshImportedHost {
  alias: string;
  sshHostId: SshHostId;
  /**
   * The row was created but holds no credential — the config named no identity
   * file, or the one it named had no readable private key. Everything else about
   * the host is right; it just cannot connect until someone supplies a secret.
   */
  needsCredential: boolean;
  /**
   * The config named no `User`, so the row's username is empty. A separate
   * missing piece from {@link needsCredential}: a host whose key was read fine is
   * still undialable without a username.
   */
  needsUsername: boolean;
}

export type IApiSshImportSkipReason = 'duplicateName' | 'duplicateEndpoint' | 'notInConfig';

export interface IApiSshImportSkipped {
  alias: string;
  reason: IApiSshImportSkipReason;
}

/** What an import did, per alias. A report — never credential material. */
export interface IApiSshImportResult {
  imported: IApiSshImportedHost[];
  skipped: IApiSshImportSkipped[];
}

const fromApiSshImportResult = (value: IApiSshImportResult): IApiSshImportResult => ({
  ...value,
  imported: value.imported.map((item) => ({
    ...item,
    sshHostId: parseSshHostId(item.sshHostId),
  })),
});

const fromApiSshStatus = (value: IApiSshStatus): IApiSshStatus => ({
  ...value,
  sshHostId: parseSshHostId(value.sshHostId),
});

export const ssh = {
  list: withResponseMap(
    httpGet<IApiSshHost[], void>('/api/ssh-hosts'),
    (items) => items.map(fromApiSshHost)
  ),
  get: withResponseMap(
    httpGet<IApiSshHost | null, { ssh_host_id: SshHostId }>(
      (p) => `/api/ssh-hosts/${p.ssh_host_id}`
    ),
    (item) => (item == null ? null : fromApiSshHost(item))
  ),
  create: withResponseMap(
    httpPost<IApiSshHost, IApiCreateSshHost>('/api/ssh-hosts'),
    fromApiSshHost
  ),
  update: withResponseMap(
    httpPut<IApiSshHost, { ssh_host_id: SshHostId; updates: IApiUpdateSshHost }>(
      (p) => `/api/ssh-hosts/${p.ssh_host_id}`,
      (p) => p.updates
    ),
    fromApiSshHost
  ),
  delete: httpDelete<void, { ssh_host_id: SshHostId }>(
    (p) => `/api/ssh-hosts/${p.ssh_host_id}`
  ),
  testConnection: httpPost<{ ok: boolean; message: string }, { ssh_host_id: SshHostId }>(
    (p) => `/api/ssh-hosts/${p.ssh_host_id}/test-connection`
  ),
  /**
   * Snapshot of every live link the caller owns. Plural on purpose: a singular
   * `/api/ssh-hosts/status` would be shadowed by the `/{ssh_host_id}` capture
   * on the same prefix.
   */
  statuses: withResponseMap(
    httpGet<IApiSshStatus[], void>('/api/ssh-hosts/statuses'),
    (items) => items.map(fromApiSshStatus)
  ),
  /** Every link transition, owner-scoped. Same payload as `statuses`. */
  onStatus: wsMappedEmitter<IApiSshStatus>('ssh.status', (raw) =>
    fromApiSshStatus(raw as IApiSshStatus)
  ),
  /** Hosts in this machine's `~/.ssh/config` that are not in the book yet. */
  importCandidates: httpGet<IApiSshConfigScan, void>('/api/ssh-hosts/import-candidates'),
  /**
   * Add the confirmed candidates to the book. Aliases only: the server re-reads
   * its own config to learn what they point at, so the client can never name a
   * file for the server to read.
   */
  importHosts: withResponseMap(
    httpPost<IApiSshImportResult, { aliases: string[] }>('/api/ssh-hosts/import'),
    fromApiSshImportResult
  ),
};

// ---------------------------------------------------------------------------
// Mini-apps — AI-generated self-contained single-file web tools, solidified
// from a conversation and reopened instantly from the sidebar library.
//
// Wire shape is snake_case (preset-style). Responses never carry the HTML
// body: the runtime loads it through the unauthenticated
// `GET /api/miniapps/{miniapp_id}/serve` route as an iframe `src`.
//
// Two copies of the document exist and the distinction is the whole reason
// `has_unpublished_changes` is on the wire: `/serve` returns the PUBLISHED
// snapshot, while a conversation edits the working copy on disk. Only `publish`
// promotes one into the other.
// ---------------------------------------------------------------------------

export interface IApiMiniApp {
  miniapp_id: MiniAppId;
  name: string;
  description: string;
  icon: string | null;
  /**
   * Provenance only — the conversation that first published this app. It is
   * deliberately left unbranded because nothing may navigate to it: a mini-app
   * outlives its source thread, so that jump is a link that rots. Its one reader
   * is the default target of 「替换已有小程序」 in the preview panel.
   */
  source_conversation_id: string | null;
  /** Size of the published snapshot in bytes; the body itself never rides list/detail responses. */
  html_size: number;
  /** Ms epoch of the last publish, or null when no document was ever promoted. */
  published_at: number | null;
  /** Derived per request: the on-disk working copy is newer than the snapshot. */
  has_unpublished_changes: boolean;
  created_at: number;
  updated_at: number;
}

/**
 * Where a mini-app's source lives on disk, as answered by
 * `POST /api/miniapps/{miniapp_id}/workspace`.
 *
 * `source_path` is the absolute `{work_dir}/miniapps/{miniapp_id}/miniapp.html`.
 * It is never an input — the server derives it from the id and runs it through
 * its escape guard — and the client only reads it back to write it into the first
 * message of an ORDINARY conversation (spec D19). No conversation is created by
 * this call.
 */
export interface IApiMiniAppWorkspace {
  source_path: string;
}

export interface IApiCreateMiniApp {
  name: string;
  description?: string;
  icon?: string;
  html: string;
  source_conversation_id?: string;
}

export interface IApiUpdateMiniApp {
  name?: string;
  description?: string;
  icon?: string;
  html?: string;
}

const fromApiMiniApp = (value: IApiMiniApp): IApiMiniApp => ({
  ...value,
  miniapp_id: parseMiniAppId(value.miniapp_id),
});

/**
 * Import intake. Supply EXACTLY ONE of `html` / `path` — the backend rejects both
 * and neither with its own message rather than guessing.
 *
 * `path` must be absolute: either one `.html`/`.htm` document, or the folder that
 * holds its `index.html`. It only works where the picker and the backend share a
 * filesystem (the desktop shell), which is why the dialog also has an inline
 * `html` flow for a WebUI browser session.
 */
export interface IApiMiniAppImportRequest {
  /** Overrides the document's `<title>` when naming the app. */
  name?: string;
  description?: string;
  icon?: string;
  html?: string;
  path?: string;
}

/**
 * How much a finding costs the user: `fatal` refuses the import, `autofix` is
 * repaired during import, `warning` only informs.
 */
export type IApiMiniAppImportSeverity = 'fatal' | 'autofix' | 'warning';

/**
 * One validation finding. `rule_id` is the join key to the UI's copy catalogue —
 * the backend deliberately sends no prose, and `detail` is structured data (the
 * offending reference, a byte count) the UI interpolates into its own sentence.
 */
export interface IApiMiniAppImportFinding {
  rule_id: string;
  severity: IApiMiniAppImportSeverity;
  detail?: string;
}

export interface IApiMiniAppImportReport {
  findings: IApiMiniAppImportFinding[];
  /** True when any finding is fatal. The import route refuses on this flag alone. */
  blocked: boolean;
}

/**
 * Answer of BOTH import routes, so one mapper serves both and a client can never
 * mistake "reported" for "adopted": `app` is present only on a real import.
 */
export interface IApiMiniAppImportResponse {
  report: IApiMiniAppImportReport;
  /** Rule ids actually repaired — never the ones the catalogue merely hoped to repair. */
  applied_fixes: string[];
  app?: IApiMiniApp;
}

const fromApiMiniAppImportResponse = (value: IApiMiniAppImportResponse): IApiMiniAppImportResponse => ({
  ...value,
  ...(value.app ? { app: fromApiMiniApp(value.app) } : {}),
});

/**
 * Recover the report from a REJECTED import.
 *
 * `POST /api/miniapps/import` answers a blocked candidate with **400 whose body
 * is still the full success envelope** (`{ success, data: { report, … } }`), so
 * the findings survive the throw: `httpRequest` reads the error body and hands it
 * to `BackendHttpError.body`. Returns `null` for anything that is not that shape
 * — a real BadRequest (`{ success: false, error }`), a 500, a transport failure —
 * which callers must then treat as a plain error.
 *
 * This is a backstop, not the main path: the dialog validates first, so a 400
 * here means the source changed underneath the user between the two calls.
 */
export function miniAppImportReportFromError(error: unknown): IApiMiniAppImportResponse | null {
  if (!isBackendHttpError(error) || error.status !== 400) return null;
  const body = error.body;
  if (!body || typeof body !== 'object') return null;
  const data = (body as { data?: unknown }).data;
  if (!data || typeof data !== 'object') return null;
  const report = (data as { report?: unknown }).report;
  if (!report || typeof report !== 'object') return null;
  if (!Array.isArray((report as { findings?: unknown }).findings)) return null;
  return fromApiMiniAppImportResponse(data as IApiMiniAppImportResponse);
}

export const miniapps = {
  list: withResponseMap(
    httpGet<IApiMiniApp[], void>('/api/miniapps'),
    (items) => items.map(fromApiMiniApp)
  ),
  get: withResponseMap(
    httpGet<IApiMiniApp | null, { miniapp_id: MiniAppId }>(
      (p) => `/api/miniapps/${p.miniapp_id}`
    ),
    (item) => (item == null ? null : fromApiMiniApp(item))
  ),
  create: withResponseMap(
    httpPost<IApiMiniApp, IApiCreateMiniApp>('/api/miniapps'),
    fromApiMiniApp
  ),
  update: withResponseMap(
    httpPut<IApiMiniApp, { miniapp_id: MiniAppId; updates: IApiUpdateMiniApp }>(
      (p) => `/api/miniapps/${p.miniapp_id}`,
      (p) => p.updates
    ),
    fromApiMiniApp
  ),
  delete: httpDelete<boolean, { miniapp_id: MiniAppId }>(
    (p) => `/api/miniapps/${p.miniapp_id}`
  ),
  /**
   * Idempotently provision this app's directory and materialize its working copy,
   * answering the ABSOLUTE source path (spec D19). Creates no conversation — the
   * caller writes the path into the first message of an ordinary one.
   *
   * No request body: the server derives the directory from the id, and the client
   * never names a path.
   */
  provisionWorkspace: httpPost<IApiMiniAppWorkspace, { miniapp_id: MiniAppId }>(
    (p) => `/api/miniapps/${p.miniapp_id}/workspace`,
    () => ({})
  ),
  /**
   * Promote the on-disk working copy into the served snapshot. 400 when there is
   * no working copy yet (nothing to publish) — iterate first.
   */
  publish: withResponseMap(
    httpPost<IApiMiniApp, { miniapp_id: MiniAppId }>(
      (p) => `/api/miniapps/${p.miniapp_id}/publish`,
      () => ({})
    ),
    fromApiMiniApp
  ),
  /**
   * Judge a candidate and write nothing. Always 200 for a readable candidate,
   * even a blocked one — the verdict is `report.blocked`, not the status.
   *
   * Registered before the `{miniapp_id}` capture on the backend, so `validate`
   * and `import` are never read as ids.
   */
  validateImport: withResponseMap(
    httpPost<IApiMiniAppImportResponse, IApiMiniAppImportRequest>('/api/miniapps/validate'),
    fromApiMiniAppImportResponse
  ),
  /**
   * Adopt a candidate. 200 carries the new `app`; a blocked candidate is a 400
   * whose body is still the report — see {@link miniAppImportReportFromError}.
   */
  importApp: withResponseMap(
    httpPost<IApiMiniAppImportResponse, IApiMiniAppImportRequest>('/api/miniapps/import'),
    fromApiMiniAppImportResponse
  ),
};

// ---------------------------------------------------------------------------
// Physical robots — ESP32 devices bound to a companion, served by the embedded
// robot gateway (`/robot/*` for the DEVICE, `/api/robots*` for this UI).
//
// A robot is keyed by `robot_id`, which is the firmware's Device-Id — a MAC
// address, not a UUIDv7 — so unlike every other entity id in this bridge it is
// deliberately NOT branded: a parser would reject every real device.
// ---------------------------------------------------------------------------

/** Live phase of one robot. `offline` = no WS session right now. */
export type IApiRobotPhase = 'offline' | 'idle' | 'listening' | 'speaking';

/** One registered robot. `companion_id === null` = paired with nobody yet. */
export interface IApiRobot {
  robot_id: string;
  name: string;
  companion_id: CompanionId | null;
  /** Firmware board type, e.g. `esp32-s3n16r8-emoji`. */
  board: string;
  firmware_version: string;
  /** RFC 3339, or null when the device has never reported in. */
  last_seen: string | null;
  /** RFC 3339. */
  created_at: string;
}

/**
 * The single wire shape for robot liveness: the `robot.status` event and the
 * `/api/robots/statuses` snapshot both carry it, so a robot cannot look
 * different depending on how the client learned about it. `changed_at` is when
 * the phase CHANGED (ms), not when it was asked — which is what makes it a
 * usable tiebreak across both arrival paths.
 */
export interface IApiRobotStatus {
  robot_id: string;
  companion_id: CompanionId | null;
  phase: IApiRobotPhase;
  changed_at: number;
}

/**
 * Where a device should be pointed, and whether it can reach us at all.
 * `ota_urls` lists one candidate per non-loopback NIC; `lan_enabled` is the LAN
 * listener's state — with it off, no device can connect no matter what it is
 * configured with.
 */
export interface IApiRobotEndpoints {
  ota_urls: string[];
  lan_enabled: boolean;
}

const fromApiRobot = (value: IApiRobot): IApiRobot => ({
  ...value,
  companion_id: value.companion_id == null ? null : parseCompanionId(value.companion_id),
});

const fromApiRobotStatus = (value: IApiRobotStatus): IApiRobotStatus => ({
  ...value,
  companion_id: value.companion_id == null ? null : parseCompanionId(value.companion_id),
});

export const robot = {
  list: withResponseMap(httpGet<{ robots: IApiRobot[] }, void>('/api/robots'), (payload) =>
    (payload.robots ?? []).map(fromApiRobot)
  ),
  /**
   * Claim the device showing `code` for `companion_id`.
   * 404 = no such code (mistyped or expired); 409 = already bound to another
   * companion. The caller surfaces the backend message verbatim.
   */
  claim: withResponseMap(
    httpPost<IApiRobot, { code: string; companion_id: CompanionId }>('/api/robots/claim'),
    fromApiRobot
  ),
  /** Rename, rebind (`companion_id`) or unbind (`companion_id: null`). */
  update: withResponseMap(
    httpPatch<
      IApiRobot,
      { robot_id: string; updates: { name?: string; companion_id?: CompanionId | null } }
    >(
      (p) => `/api/robots/${p.robot_id}`,
      (p) => p.updates
    ),
    fromApiRobot
  ),
  /** Revoke the device token and forget the record; the device becomes new again. */
  remove: httpDelete<void, { robot_id: string }>((p) => `/api/robots/${p.robot_id}`),
  /** Snapshot of every robot's phase. Plural for the same reason ssh statuses is. */
  statuses: withResponseMap(
    httpGet<{ statuses: IApiRobotStatus[] }, void>('/api/robots/statuses'),
    (payload) => (payload.statuses ?? []).map(fromApiRobotStatus)
  ),
  endpoints: httpGet<IApiRobotEndpoints, void>('/api/robots/endpoints'),
  /** Every phase transition, owner-scoped. Same payload as `statuses`. */
  onStatus: wsMappedEmitter<IApiRobotStatus>('robot.status', (raw) =>
    fromApiRobotStatus(raw as IApiRobotStatus)
  ),
};

// ---------------------------------------------------------------------------
// Database — routed to conversation/message endpoints
// ---------------------------------------------------------------------------

export type PaginatedResult<T> = {
  items: T[];
  total: number;
  has_more: boolean;
};

export const database = {
  getConversationMessages: withResponseMap(
    httpGet<
      PaginatedResult<StoredMessageResponse>,
      {
        conversation_id: ConversationId;
        page?: number;
        page_size?: number;
        order?: string;
        content_mode?: 'compact' | 'full';
        // Keyset cursor for incremental history loading: '' = newest window,
        // '<created_at>:<message_id>' = the page strictly older than that
        // persisted message. When
        // set (incl. ''), the backend ignores page/offset pagination.
        cursor?: string;
        // One LOCAL calendar day (`YYYYMMDD`), oldest-first and complete: the
        // backend decides the day boundary (the same one that partitions
        // companion session digests), so a reader never re-derives days in a
        // browser whose timezone may differ. Mutually exclusive with `cursor`.
        day?: string;
      }
    >((p) => {
      const params = new URLSearchParams();
      params.set('page', String(p.page ?? 1));
      params.set('page_size', String(p.page_size ?? 50));
      if (p.order) params.set('order', p.order);
      if (p.content_mode) params.set('content_mode', p.content_mode);
      // Send even an empty cursor (the "newest window" request) — distinct from
      // omitting it, which selects offset pagination.
      if (p.cursor !== undefined) params.set('cursor', p.cursor);
      if (p.day) params.set('day', p.day);
      return `/api/conversations/${p.conversation_id}/messages?${params.toString()}`;
    }),
    (page) => ({ ...page, items: page.items.map(fromApiStoredMessage) })
  ),
  getConversationMessage: withResponseMap(
    httpGet<
      StoredMessageResponse,
      { conversation_id: ConversationId; message_id: MessageId }
    >((p) => `/api/conversations/${p.conversation_id}/messages/${encodeURIComponent(p.message_id)}`),
    fromApiStoredMessage
  ),
  getUserConversations: withResponseMap(
    httpGet<PaginatedResult<unknown>, { cursor?: string; limit?: number }>(
      (p) => {
        const params = new URLSearchParams();
        if (p.cursor) params.set('cursor', p.cursor);
        if (p.limit) params.set('limit', String(p.limit));
        const qs = params.toString();
        return `/api/conversations${qs ? `?${qs}` : ''}`;
      }
    ),
    fromApiPaginatedConversations
  ),
  searchConversationMessages: withResponseMap(
    httpGet<PaginatedResult<ApiMessageSearchItem>, { keyword: string; page?: number; page_size?: number }>(
      (p) =>
        `/api/messages/search?keyword=${encodeURIComponent(p.keyword)}&page=${p.page ?? 1}&page_size=${p.page_size ?? 50}`
    ),
    fromApiSearchResult
  ),
};

// ---------------------------------------------------------------------------
// Preview History — routed to /api/preview-history/*
// ---------------------------------------------------------------------------

function mapPreviewTarget(target: PreviewHistoryTarget): Record<string, unknown> {
  return {
    ...target,
    content_type: target.contentType,
    contentType: undefined,
  };
}

function fromApiPreviewSnapshot(value: PreviewSnapshotInfo): PreviewSnapshotInfo {
  const raw = value as PreviewSnapshotInfo & { id?: unknown };
  if (raw.id !== undefined) {
    throw new TypeError('preview snapshot payload must use snapshot_id, not id');
  }
  return {
    ...value,
    snapshot_id: parsePreviewSnapshotId(value.snapshot_id),
  };
}

export const previewHistory = {
  list: withResponseMap(
    httpPost<PreviewSnapshotInfo[], { target: PreviewHistoryTarget }>('/api/preview-history/list', (p) => ({
      target: mapPreviewTarget(p.target),
    })),
    (snapshots) => snapshots.map(fromApiPreviewSnapshot)
  ),
  save: withResponseMap(
    httpPost<PreviewSnapshotInfo, { target: PreviewHistoryTarget; content: string }>(
      '/api/preview-history/save',
      (p) => ({
        target: mapPreviewTarget(p.target),
        content: p.content,
      })
    ),
    fromApiPreviewSnapshot
  ),
  getContent: withResponseMap(
    httpPost<
      { snapshot: PreviewSnapshotInfo; content: string } | null,
      { target: PreviewHistoryTarget; snapshot_id: PreviewSnapshotInfo['snapshot_id'] }
    >('/api/preview-history/get-content', (p) => ({
      target: mapPreviewTarget(p.target),
      snapshot_id: p.snapshot_id,
    })),
    (response) =>
      response
        ? {
            ...response,
            snapshot: fromApiPreviewSnapshot(response.snapshot),
          }
        : null
  ),
};

// Preview panel
export const preview = {
  open: wsEmitter<{
    content: string;
    content_type: import('../types/office/preview').PreviewContentType;
    metadata?: {
      title?: string;
      file_name?: string;
    };
  }>('preview.open'),
};

// ---------------------------------------------------------------------------
// Office Previews — routed to /api/*-preview/*
// ---------------------------------------------------------------------------

export const pptPreview = {
  start: httpPost<PreviewUrlResponse, { file_path: string; workspace?: string }>('/api/ppt-preview/start'),
  stop: httpPost<void, { capability: string }>('/api/ppt-preview/stop'),
  status: wsEmitter<{
    state: 'starting' | 'installing' | 'ready' | 'error';
    message?: string;
  }>('ppt-preview.status'),
};

export const wordPreview = {
  start: httpPost<PreviewUrlResponse, { file_path: string; workspace?: string }>('/api/word-preview/start'),
  stop: httpPost<void, { capability: string }>('/api/word-preview/stop'),
  status: wsEmitter<{
    state: 'starting' | 'installing' | 'ready' | 'error';
    message?: string;
  }>('word-preview.status'),
};

export const excelPreview = {
  start: httpPost<PreviewUrlResponse, { file_path: string; workspace?: string }>('/api/excel-preview/start'),
  stop: httpPost<void, { capability: string }>('/api/excel-preview/stop'),
  status: wsEmitter<{
    state: 'starting' | 'installing' | 'ready' | 'error';
    message?: string;
  }>('excel-preview.status'),
};

// ---------------------------------------------------------------------------
// Deep Link — stays IPC (Electron protocol handler)
// ---------------------------------------------------------------------------

export const deepLink = {
  received: shellEmitter<{ action: string; params: Record<string, string> }>((cb) => subscribeDeepLink(cb)),
};

// ---------------------------------------------------------------------------
// Window Controls — stays IPC (Electron-native)
// ---------------------------------------------------------------------------

export const windowControls = {
  minimize: shellProvider<void, void>(() => tauriWindowMinimize(), undefined),
  maximize: shellProvider<void, void>(() => tauriWindowMaximize(), undefined),
  unmaximize: shellProvider<void, void>(() => tauriWindowUnmaximize(), undefined),
  // Double-click-titlebar entry: a single native toggle (avoids a race between a
  // separate isMaximized read and maximize/unmaximize). Windows/Linux only — on
  // macOS the OS handles titlebar double-click on the native chrome.
  toggleMaximize: shellProvider<void, void>(() => tauriWindowToggleMaximize(), undefined),
  close: shellProvider<void, void>(() => tauriWindowClose(), undefined),
  isMaximized: shellProvider<boolean, void>(() => tauriWindowIsMaximized(), false),
  maximizedChanged: shellEmitter<{ is_maximized: boolean }>((cb) => subscribeWindowMaximized(cb)),
};

// ---------------------------------------------------------------------------
// System Settings — routed to /api/settings/* unless they need Electron-native side effects.
// ---------------------------------------------------------------------------

export const systemSettings = {
  getNotificationEnabled: httpGet<boolean, void>('/api/settings/client?key=notificationEnabled'),
  setNotificationEnabled: httpPut<void, { enabled: boolean }>('/api/settings/client', (p) => ({
    notificationEnabled: p.enabled,
  })),
  getCronNotificationEnabled: httpGet<boolean, void>('/api/settings/client?key=cronNotificationEnabled'),
  setCronNotificationEnabled: httpPut<void, { enabled: boolean }>('/api/settings/client', (p) => ({
    cronNotificationEnabled: p.enabled,
  })),
  getKeepAwake: httpGet<boolean, void>('/api/settings/client?key=keepAwake'),
  setKeepAwake: httpPut<void, { enabled: boolean }>('/api/settings/client', (p) => ({ keepAwake: p.enabled })),
  changeLanguage: httpPatch<void, { language: string }>('/api/settings', (p) => ({ language: p.language })),
  languageChanged: wsEmitter<{ language: string }>('system-settings:language-changed'),
  getSaveUploadToWorkspace: httpGet<boolean, void>('/api/settings/client?key=saveUploadToWorkspace'),
  setSaveUploadToWorkspace: httpPut<void, { enabled: boolean }>('/api/settings/client', (p) => ({
    saveUploadToWorkspace: p.enabled,
  })),
  getAutoPreviewOfficeFiles: httpGet<boolean, void>('/api/settings/client?key=autoPreviewOfficeFiles'),
  setAutoPreviewOfficeFiles: httpPut<void, { enabled: boolean }>('/api/settings/client', (p) => ({
    autoPreviewOfficeFiles: p.enabled,
  })),
};

// ---------------------------------------------------------------------------
// Computer-use OS permissions — macOS TCC (Accessibility / Screen Recording).
// Routed to the in-process backend, which probes/triggers the HOST process's
// OWN grants, so `get` is the authoritative answer to "did my grant take effect
// for the running app?" — a visibly-on System Settings toggle bound to a stale
// code identity reports `false` here. Off macOS the booleans are null.
// ---------------------------------------------------------------------------

export type ComputerPermissionKind = 'accessibility' | 'screen_recording';

export interface ComputerPermissionStatus {
  accessibility: boolean | null;
  screen_recording: boolean | null;
  platform: 'macos' | 'windows' | 'linux' | 'other';
  app_label: string;
}

export const computerPermissions = {
  /** Live grant state for the running host process (safe to poll). */
  get: httpGet<ComputerPermissionStatus, void>('/api/computer/permissions'),
  /** Trigger the macOS prompt + register the app in the list; returns post-call status. */
  request: httpPost<ComputerPermissionStatus, { kind: ComputerPermissionKind }>(
    '/api/computer/permissions/request'
  ),
  /** Deep-link to the exact System Settings privacy pane for `kind`. */
  openSettings: httpPost<void, { kind: ComputerPermissionKind }>('/api/computer/permissions/open-settings'),
};

// ---------------------------------------------------------------------------
// Computer history — local activity capture (foreground app / window / URL
// segments). Read surface mirrors the backend `computer_history_*` capability
// family (design draft §5): status, segment list, per-app usage rollup, the
// feature toggle (a `feature.computer_history` client preference) and a
// destructive purge that needs explicit user confirmation in the UI.
// ---------------------------------------------------------------------------

export type ComputerHistoryState = 'stopped' | 'running' | 'paused';
export type ComputerHistoryPermission = 'granted' | 'denied' | 'unknown';

export interface IComputerHistoryStorageStatus {
  segments: number;
  approx_bytes: number;
  path: string;
}

/** Chat.db analytics availability (optional field — may be absent on older builds). */
export interface IComputerHistoryChatAnalytics {
  available: boolean;
  db_path: string;
}

export interface IComputerHistoryStatus {
  enabled: boolean;
  state: ComputerHistoryState;
  permission: ComputerHistoryPermission;
  paused_until: string | null;
  storage: IComputerHistoryStorageStatus;
  chat_analytics?: IComputerHistoryChatAnalytics | null;
}

export interface IComputerHistorySegment {
  event_id: string;
  app_name: string;
  window_title: string | null;
  browser_url: string | null;
  started_at_ms: number;
  ended_at_ms: number;
  source: string;
}

export interface IComputerHistoryListParams {
  from_ms?: number;
  to_ms?: number;
  limit?: number;
}

export interface IComputerHistoryAppUsageRow {
  app_name: string;
  total_ms: number;
  segment_count: number;
}

export type ComputerHistoryWindow = 'today' | 'yesterday' | 'last_7_days' | 'this_week';

export const computerHistory = {
  status: httpGet<IComputerHistoryStatus, void>('/api/computer-history/status'),
  list: httpGet<IComputerHistorySegment[], IComputerHistoryListParams>((p) => {
    const query = new URLSearchParams();
    if (p.from_ms != null) query.set('from_ms', String(p.from_ms));
    if (p.to_ms != null) query.set('to_ms', String(p.to_ms));
    if (p.limit != null) query.set('limit', String(p.limit));
    const qs = query.toString();
    return `/api/computer-history/segments${qs ? `?${qs}` : ''}`;
  }),
  appUsage: httpGet<IComputerHistoryAppUsageRow[], IComputerHistoryListParams>((p) => {
    const query = new URLSearchParams();
    if (p.from_ms != null) query.set('from_ms', String(p.from_ms));
    if (p.to_ms != null) query.set('to_ms', String(p.to_ms));
    if (p.limit != null) query.set('limit', String(p.limit));
    const qs = query.toString();
    return `/api/computer-history/app-usage${qs ? `?${qs}` : ''}`;
  }),
  setEnabled: httpPost<{ ok: boolean }, { enabled: boolean }>('/api/computer-history/settings', (p) => ({
    enabled: p.enabled,
  })),
  purge: httpDelete<{ deleted: number }, { before_ms?: number }>((p) =>
    p && p.before_ms != null ? `/api/computer-history/segments?before_ms=${p.before_ms}` : '/api/computer-history/segments'
  ),
};

// ---------------------------------------------------------------------------
// System events — global WS broadcasts owned by the backend
// ---------------------------------------------------------------------------

// (browser-automation runtime provisioning was removed with the native CDP
// engine — the self-contained engine acquires Chrome on demand, so there is no
// longer a `system.provisioning` broadcast to surface.)

// ---------------------------------------------------------------------------
// Notification — stays IPC (Electron-native Notification API)
// ---------------------------------------------------------------------------

export type INotificationOptions = {
  title: string;
  body: string;
  icon?: string;
  conversation_id?: ConversationId;
};

export const notification = {
  show: shellProvider<void, INotificationOptions>(
    (opts) =>
      tauriSendNotification({
        title: opts.title,
        body: opts.body,
        icon: opts.icon,
      }),
    undefined
  ),
  // DEGRADE_STUB: click→navigate needs a Rust notification-action listener that
  // emits a Tauri event (see electron-removal-plan C2); inert until then.
  clicked: noopEmitter<{ conversation_id?: ConversationId }>(),
};

// ---------------------------------------------------------------------------
// Task management — stubbed (internal process management)
// ---------------------------------------------------------------------------

export const task = {
  stopAll: stubProvider<{ success: boolean; count: number }, void>('task.stopAll', { success: true, count: 0 }),
  getRunningCount: stubProvider<{ success: boolean; count: number }, void>('task.getRunningCount', {
    success: true,
    count: 0,
  }),
};

// ---------------------------------------------------------------------------
// WebUI — mix: start/stop/getStatus/statusChanged stay IPC (Electron-only
// lifecycle owned by the main process, can't run in backend); credential
// operations route to backend /api/webui/* under local-mode.
// ---------------------------------------------------------------------------

export interface IWebUIStatus {
  running: boolean;
  port: number;
  allowRemote: boolean;
  localUrl: string;
  networkUrl?: string;
  /** A quick-access URL per non-loopback NIC (routing-preferred first). */
  networkUrls?: string[];
  lanIP?: string;
  adminUsername: string;
  /** Whether a real admin password is stored (non-empty hash). Lets the UI
   *  distinguish "credential set (hidden)" from "never provisioned" even when
   *  the LAN server is stopped, so a persisted password is not read as lost. */
  passwordSet?: boolean;
  initialPassword?: string;
  /** Set when a start attempt failed (e.g. could not bind the port). */
  error?: string;
}

export const webui = {
  /**
   * Capability bit: can this runtime start/stop the LAN listener and report its
   * real status? True in the Tauri desktop shell (the embedded backend owns the
   * LAN-listener lifecycle via the `webui_*` commands). False in a WebUI
   * browser — that page IS served by the LAN listener, so it cannot control it.
   */
  lifecycleSupported: typeof window !== 'undefined' && Boolean((window as { __backendPort?: number }).__backendPort),
  getStatus: shellProvider<IWebUIStatus, void>(() => tauriWebuiGetStatus<IWebUIStatus>(), {
    running: false,
    port: 0,
    allowRemote: false,
    localUrl: '',
    adminUsername: '',
  }),
  // Enabling binds the LAN listener (0.0.0.0); the backend returns the full
  // status (running + error + lanIP + one-time initialPassword).
  start: shellProvider<IWebUIStatus, void>(() => tauriWebuiStart<IWebUIStatus>(), {
    running: false,
    port: 0,
    allowRemote: false,
    localUrl: '',
    adminUsername: '',
    error: 'desktop lifecycle unavailable',
  }),
  stop: shellProvider<void, void>(() => tauriWebuiStop<void>(), undefined),
  statusChanged: shellEmitter<{
    running: boolean;
    port?: number;
    localUrl?: string;
    networkUrl?: string;
    networkUrls?: string[];
    lanIP?: string;
    adminUsername?: string;
    passwordSet?: boolean;
    initialPassword?: string;
  }>((cb) => subscribeWebuiStatus(cb)),
  changePassword: httpPost<void, { newPassword: string }>('/api/webui/change-password', (p) => ({
    new_password: p.newPassword,
  })),
  changeUsername: httpPost<{ username: string }, { newUsername: string }>('/api/webui/change-username', (p) => ({
    new_username: p.newUsername,
  })),
  /**
   * Authenticated self-service credential changes for WebUI browser sessions.
   * Unlike the local-trust `/api/webui/*` variants above (desktop shell only,
   * possession = auth), these verify the CURRENT password and work for a
   * remote login — docker users change the login without touching container
   * parameters. `changePassword` rotates the JWT secret server-side: every
   * session (including this one) is invalidated and the user must sign in
   * again with the new password.
   */
  account: {
    changePassword: httpPost<void, { currentPassword: string; newPassword: string }>(
      '/api/auth/change-password',
      (p) => ({
        current_password: p.currentPassword,
        new_password: p.newPassword,
      })
    ),
    changeUsername: httpPost<{ username: string }, { currentPassword: string; newUsername: string }>(
      '/api/auth/change-username',
      (p) => ({
        current_password: p.currentPassword,
        new_username: p.newUsername,
      })
    ),
  },
  generateQRToken: httpPost<{ token: string; expires_at_ms: number }, void>('/api/webui/generate-qr-token'),
  /**
   * Installation-scoped Remote access token (local-trust-gated; desktop only).
   * Mint returns plaintext exactly once; the backend persists only its hash.
   * The credential authenticates NomiFun Desktop and never selects a companion.
   */
  instanceAccessToken: {
    status: httpGet<{ configured: boolean }, void>('/api/webui/access-token'),
    mint: httpPost<{ token: string; warning?: string }, void>('/api/webui/access-token'),
    revoke: httpDelete<{ configured: boolean }, void>('/api/webui/access-token'),
  },
};

export type IRelayPairingStatus = TauriRelayPairingStatus;
export type IRelayPairingBootstrapRequest = TauriRelayPairingBootstrapRequest;

export const relayPairing = {
  bootstrap: shellProvider<IRelayPairingStatus, IRelayPairingBootstrapRequest>(
    (request) => tauriRelayPairingBootstrap(request),
    { state: 'disconnected' }
  ),
  getStatus: shellProvider<IRelayPairingStatus, void>(
    () => tauriRelayPairingGetStatus(),
    { state: 'disconnected' }
  ),
  stop: shellProvider<IRelayPairingStatus, void>(
    () => tauriRelayPairingStop(),
    { state: 'disconnected' }
  ),
  restart: shellProvider<IRelayPairingStatus, void>(
    () => tauriRelayPairingRestart(),
    { state: 'disconnected' }
  ),
  disconnect: shellProvider<IRelayPairingStatus, void>(
    () => tauriRelayPairingDisconnect(),
    { state: 'disconnected' }
  ),
};

// ---------------------------------------------------------------------------
// Cron — routed to /api/cron/*
// ---------------------------------------------------------------------------

function fromApiCronJob(job: ICronJob): ICronJob {
  if (job.metadata.agent_type === 'nomi' && job.metadata.agent_config?.backend != null) {
    throw new TypeError(
      'Nomi cron agent_config.backend is not accepted; use agent_config.provider_id'
    );
  }
  return {
    ...job,
    cron_job_id: parseCronJobId(job.cron_job_id),
    metadata: {
      ...job.metadata,
      conversation_id: parseOptionalEntityId('conversation', job.metadata.conversation_id),
      ...(job.metadata.agent_config == null
        ? {}
        : {
            agent_config: {
              ...job.metadata.agent_config,
              custom_agent_id:
                job.metadata.agent_config.custom_agent_id == null
                  ? undefined
                  : parseAgentId(job.metadata.agent_config.custom_agent_id),
              preset_id:
                job.metadata.agent_config.preset_id == null
                  ? undefined
                  : parsePresetReference(job.metadata.agent_config.preset_id),
              preset_snapshot:
                job.metadata.agent_config.preset_snapshot == null
                  ? undefined
                  : fromApiResolvedPresetSnapshot(job.metadata.agent_config.preset_snapshot),
              provider_id:
                job.metadata.agent_config.provider_id == null
                  ? undefined
                  : parseProviderId(job.metadata.agent_config.provider_id),
            },
          }),
    },
  };
}

function fromApiCronJobRun(run: ICronJobRun): ICronJobRun {
  return {
    ...run,
    cron_job_run_id: parseCronJobRunId(run.cron_job_run_id),
    cron_job_id: parseCronJobId(run.cron_job_id),
  };
}

export const cron = {
  listJobs: withResponseMap(httpGet<ICronJob[], void>('/api/cron/jobs'), (jobs) => jobs.map(fromApiCronJob)),
  listJobsByConversation: withResponseMap(
    httpGet<ICronJob[], { conversation_id: ConversationId }>(
      (p) => `/api/cron/jobs?conversation_id=${encodeURIComponent(p.conversation_id)}`
    ),
    (jobs) => jobs.map(fromApiCronJob)
  ),
  getJob: withResponseMap(httpGet<ICronJob | null, { cron_job_id: CronJobId }>((p) => `/api/cron/jobs/${p.cron_job_id}`), (job) => job ? fromApiCronJob(job) : null),
  addJob: withResponseMap(httpPost<ICronJob, ICreateCronJobParams>('/api/cron/jobs'), fromApiCronJob),
  updateJob: withResponseMap(httpPut<ICronJob, { cron_job_id: CronJobId; updates: IUpdateCronJobParams }>(
    (p) => `/api/cron/jobs/${p.cron_job_id}`,
    (p) => ({
      name: p.updates.name,
      description: p.updates.description,
      enabled: p.updates.enabled,
      schedule: p.updates.schedule,
      message: p.updates.message,
      agent_config: p.updates.agent_config,
      conversation_title: p.updates.conversation_title,
      max_retries: p.updates.max_retries,
    })
  ), fromApiCronJob),
  removeJob: httpDelete<void, { cron_job_id: CronJobId }>((p) => `/api/cron/jobs/${p.cron_job_id}`),
  runNow: {
    provider: () => {},
    invoke: async (p: {
      cron_job_id: CronJobId;
      idempotency_key: string;
    }): Promise<{ conversation_id: ConversationId }> => {
      const idempotencyKey = requireConversationIdempotencyKey(p.idempotency_key);
      const value = await httpRequest<{ conversation_id: unknown }>(
        'POST',
        `/api/cron/jobs/${p.cron_job_id}/run`,
        undefined,
        { idempotencyKey }
      );
      return { conversation_id: parseConversationId(value.conversation_id) };
    },
  },
  listRuns: withResponseMap(httpGet<ICronJobRun[], { cron_job_id: CronJobId }>((p) => `/api/cron/jobs/${p.cron_job_id}/runs`), (runs) => runs.map(fromApiCronJobRun)),
  saveSkill: httpPost<void, { cron_job_id: CronJobId; content: string }>(
    (p) => `/api/cron/jobs/${p.cron_job_id}/skill`,
    (p) => ({ content: p.content })
  ),
  hasSkill: withResponseMap(
    httpGet<{ has_skill: boolean }, { cron_job_id: CronJobId }>((p) => `/api/cron/jobs/${p.cron_job_id}/skill`),
    (data) => Boolean(data?.has_skill)
  ),
  deleteSkill: httpDelete<void, { cron_job_id: CronJobId }>((p) => `/api/cron/jobs/${p.cron_job_id}/skill`),
  onJobCreated: wsMappedEmitter<ICronJob>('cron.job-created', fromApiCronJob),
  onJobUpdated: wsMappedEmitter<ICronJob>('cron.job-updated', fromApiCronJob),
  onJobRemoved: wsMappedEmitter<{ cron_job_id: CronJobId }>('cron.job-removed', (value) => ({ cron_job_id: parseCronJobId(value.cron_job_id) })),
  onJobExecuted: wsMappedEmitter<{
    cron_job_id: CronJobId;
    status: 'ok' | 'error' | 'skipped' | 'missed';
    error?: string;
  }>('cron.job-executed', (value) => ({ ...value, cron_job_id: parseCronJobId(value.cron_job_id) })),
};

// ---------------------------------------------------------------------------
// Cron types (re-exported for consumers)
// ---------------------------------------------------------------------------

export type ICronSchedule =
  | { kind: 'at'; at_ms: number; description: string }
  | { kind: 'every'; every_ms: number; description: string }
  | { kind: 'cron'; expr: string; tz?: string; description: string };

export type ICronJobRunStatus = 'ok' | 'error' | 'skipped' | 'missed';

export interface ICronJob {
  cron_job_id: CronJobId;
  name: string;
  description?: string;
  enabled: boolean;
  schedule: ICronSchedule;
  message: string;
  execution_mode: 'existing' | 'new_conversation';
  metadata: {
    /** Absent until an unbound task materializes its first conversation. */
    conversation_id?: ConversationId;
    conversation_title?: string;
    agent_type: string;
    created_by: 'user' | 'agent';
    created_at: number;
    updated_at: number;
    agent_config?: ICronAgentConfig;
  };
  state: {
    next_run_at_ms?: number;
    last_run_at_ms?: number;
    last_status?: ICronJobRunStatus;
    last_error?: string;
    run_count: number;
    retry_count: number;
    max_retries: number;
  };
}

export interface ICronJobRun {
  cron_job_run_id: CronJobRunId;
  cron_job_id: CronJobId;
  executed_at_ms: number;
  status: ICronJobRunStatus;
}

export interface ICronAgentConfig {
  /** Agent backend label; absent for jobs without one. */
  backend?: string;
  name: string;
  cli_path?: string;
  /** Stable AgentRegistry identity required for every non-Nomi new conversation. */
  custom_agent_id?: AgentId;
  preset_id?: PresetReference;
  /** Frozen server-owned preset lineage returned by the API. */
  preset_revision?: number;
  preset_snapshot?: ResolvedPresetSnapshot;
  mode?: string;
  model?: string;
  /** Nomi logical reference to the provider business entity. */
  provider_id?: ProviderId;
  config_options?: Record<string, string>;
  workspace?: string;
  /** Clear the agent context before each scheduled run (existing-conversation jobs only). */
  clear_context_each_run?: boolean;
}

export interface ICreateCronJobParams {
  name: string;
  description?: string;
  schedule: ICronSchedule;
  prompt?: string;
  message?: string;
  /** Only specified-conversation creation supplies this; other modes start unbound. */
  conversation_id?: ConversationId;
  conversation_title?: string;
  agent_type: string;
  created_by: 'user' | 'agent';
  execution_mode?: 'existing' | 'new_conversation';
  agent_config?: ICronAgentConfig;
}

/**
 * Mutable fields accepted by PUT /api/cron/jobs/{cron_job_id}.
 *
 * Keep this separate from ICronJob: response-only and creation-only fields
 * (especially execution_mode) must never leak into the strict update DTO.
 */
export interface IUpdateCronJobParams {
  name?: string;
  description?: string;
  enabled?: boolean;
  schedule?: ICronSchedule;
  message?: string;
  agent_config?: ICronAgentConfig;
  conversation_title?: string;
  max_retries?: number;
}

// ---------------------------------------------------------------------------
// Terminal — routed to /api/terminals/*
// ---------------------------------------------------------------------------

export interface ITerminalSession {
  /** Canonical terminal entity id on the wire. */
  terminal_id: TerminalId;
  /**
   * Conversation that owns an agent-created terminal. Absent for standalone
   * terminals created explicitly by the user.
   */
  owner_conversation_id?: ConversationId;
  name: string;
  cwd: string;
  /** 派生字段（不落库）：cwd 等于或位于默认工作路径之下 / Derived: cwd equals or sits under the backend default work dir. */
  is_default_workpath?: boolean;
  command: string;
  args: string[];
  backend?: string;
  mode?: string;
  cols: number;
  rows: number;
  created_at: number;
  updated_at: number;
  last_status: 'running' | 'exited' | 'error';
  exit_code?: number;
  pinned?: boolean;
  pinned_at?: number;
  /** Base64 scrollback snapshot — present only on single-session GET. */
  scrollback_b64?: string;
}

export interface ICreateTerminalParams {
  name?: string;
  cwd: string;
  command: string;
  args?: string[];
  env?: Record<string, string>;
  backend?: string;
  mode?: string;
  cols?: number;
  rows?: number;
  /** 推迟到首个 resize(携带真实尺寸)再 spawn PTY,使全屏 TUI(claude)首帧即按正确尺寸绘制,避免「进入即花屏、需手动调尺寸」 / Defer the PTY spawn until the first resize carries the real size. */
  defer_spawn?: boolean;
  /** 创建即绑定的知识库 id；启动时挂载到 {cwd}/.nomi/knowledge/ / Knowledge bases bound at creation, mounted before the PTY spawns. */
  knowledge_base_ids?: string[];
}

export interface IMcpRegisterTemplate {
  claude_cmd: string;
  claude_json: string;
  codex_toml: string;
  gemini_json: string;
}

export interface IRegisterKnowledgeOutcome {
  written_path: string;
  scope: string;
  note?: string;
}

export interface IUnregisterKnowledgeOutcome {
  path: string;
  removed: boolean;
}

export type KnowledgeCliFamily = 'claude' | 'codex' | 'gemini';

export interface IKnowledgeGlobalRegistrationStatus {
  claude: boolean | null;
  codex: boolean | null;
  gemini: boolean | null;
}

type ApiTerminalSession = Omit<ITerminalSession, 'terminal_id' | 'owner_conversation_id'> & {
  terminal_id: unknown;
  owner_conversation_id?: unknown;
};

const fromApiTerminalSession = (raw: ApiTerminalSession): ITerminalSession => ({
  ...raw,
  terminal_id: parseTerminalId(raw.terminal_id),
  owner_conversation_id: parseOptionalEntityId('conversation', raw.owner_conversation_id),
});

export const terminal = {
  list: withResponseMap(
    httpGet<ApiTerminalSession[], void>('/api/terminals'),
    (items) => items.map(fromApiTerminalSession),
  ),
  listConversation: withResponseMap(
    httpGet<ApiTerminalSession[], { conversation_id: ConversationId }>(
      (p) => `/api/conversations/${p.conversation_id}/terminals`,
    ),
    (items) => items.map(fromApiTerminalSession),
  ),
  get: withResponseMap(
    httpGet<ApiTerminalSession, { terminal_id: TerminalId }>(
      (p) => `/api/terminals/${p.terminal_id}`,
      { timeoutMs: 10_000 }
    ),
    fromApiTerminalSession,
  ),
  create: withResponseMap(
    httpPost<ApiTerminalSession, ICreateTerminalParams>('/api/terminals'),
    fromApiTerminalSession,
  ),
  mcpRegisterTemplate: httpGet<IMcpRegisterTemplate, void>('/api/terminals/mcp-register-template'),
  registerKnowledge: httpPost<
    IRegisterKnowledgeOutcome,
    { cwd: string; family: KnowledgeCliFamily }
  >('/api/terminals/register-knowledge'),
  registerKnowledgeGlobal: httpPost<
    IRegisterKnowledgeOutcome,
    { family: KnowledgeCliFamily }
  >('/api/terminals/register-knowledge-global'),
  unregisterKnowledgeGlobal: httpPost<
    IUnregisterKnowledgeOutcome,
    { family: KnowledgeCliFamily }
  >('/api/terminals/unregister-knowledge-global'),
  knowledgeGlobalStatus: httpGet<IKnowledgeGlobalRegistrationStatus, void>(
    '/api/terminals/knowledge-global-status'
  ),
  input: httpPost<void, { terminal_id: TerminalId; data_b64: string }>(
    (p) => `/api/terminals/${p.terminal_id}/input`,
    (p) => ({ data_b64: p.data_b64 })
  ),
  resize: httpPost<void, { terminal_id: TerminalId; cols: number; rows: number }>(
    (p) => `/api/terminals/${p.terminal_id}/resize`,
    (p) => ({ cols: p.cols, rows: p.rows }),
    // Deferred activation is serialized and resize is idempotent, so a client
    // deadline can safely turn a hung request into the Xterm retry/error path.
    { timeoutMs: 6_000 }
  ),
  kill: httpPost<void, { terminal_id: TerminalId }>((p) => `/api/terminals/${p.terminal_id}/kill`),
  relaunch: withResponseMap(
    httpPost<ApiTerminalSession, { terminal_id: TerminalId }>(
      (p) => `/api/terminals/${p.terminal_id}/relaunch`
    ),
    fromApiTerminalSession,
  ),
  /** 把会话原地回退为干净的登录 shell(杀掉卡死的 claude/codex 并以 $SHELL 重启同一会话) / Fall back to a clean login shell in place. */
  relaunchShell: withResponseMap(
    httpPost<ApiTerminalSession, { terminal_id: TerminalId }>(
      (p) => `/api/terminals/${p.terminal_id}/relaunch-shell`
    ),
    fromApiTerminalSession,
  ),
  update: withResponseMap(
    httpPatch<ApiTerminalSession, { terminal_id: TerminalId; name?: string; pinned?: boolean }>(
      (p) => `/api/terminals/${p.terminal_id}`,
      (p) => ({ name: p.name, pinned: p.pinned }),
    ),
    fromApiTerminalSession,
  ),
  remove: httpDelete<void, { terminal_id: TerminalId }>(
    (p) => `/api/terminals/${p.terminal_id}`
  ),
  onOutput: wsMappedEmitter<{ terminal_id: TerminalId; data_b64: string }>(
    'terminal.output',
    (raw) => {
      const event = raw as { terminal_id: unknown; data_b64: string };
      return { ...event, terminal_id: parseTerminalId(event.terminal_id) };
    },
  ),
  onExit: wsMappedEmitter<{ terminal_id: TerminalId; exit_code?: number }>(
    'terminal.exit',
    (raw) => {
      const event = raw as { terminal_id: unknown; exit_code?: number };
      return { ...event, terminal_id: parseTerminalId(event.terminal_id) };
    },
  ),
  onCreated: wsMappedEmitter<ITerminalSession, ApiTerminalSession>('terminal.created', (raw) =>
    fromApiTerminalSession(raw),
  ),
  onUpdated: wsMappedEmitter<ITerminalSession, ApiTerminalSession>('terminal.updated', (raw) =>
    fromApiTerminalSession(raw),
  ),
  onRemoved: wsMappedEmitter<{ terminal_id: TerminalId }>('terminal.removed', (raw) => {
    const event = raw as { terminal_id: unknown };
    return { terminal_id: parseTerminalId(event.terminal_id) };
  }),
  /** 在 WebSocket 断线重连成功后触发(本地合成事件,非服务端推送)。XtermView 借此 reset
   *  并重放 scrollback,修复断线期间丢失的重绘帧造成的乱码 / Fires after the WS reconnects
   *  (local synthetic event) so a view can reset + replay the scrollback it missed. */
  onReconnected: wsEmitter<undefined>('ws.reconnected'),
  // Uses httpRequest directly (instead of httpGet + withResponseMap) because the
  // response mapper needs `cwd` from params to build fullPath/relativePath, and
  // withResponseMap's map function does not receive the original params. Treats
  // `cwd` as the workspace root — same {name,type}[] wire shape as the
  // conversation workspace endpoint, so the workspace mapper is reused as-is.
  getWorkspace: {
    provider: () => {},
    invoke: (async (p: { terminal_id: TerminalId; cwd: string; path: string; search?: string }) => {
      const rel = absoluteToRelativePath(p.path, p.cwd);
      const url = `/api/terminals/${p.terminal_id}/workspace?path=${encodeURIComponent(rel)}${p.search ? `&search=${encodeURIComponent(p.search)}` : ''}`;
      const raw = await httpRequest<Array<{ name: string; type: string }>>('GET', url);
      return fromBackendWorkspaceList(raw, p.cwd, rel);
    }) as (p: { terminal_id: TerminalId; cwd: string; path: string; search?: string }) => Promise<IDirOrFile[]>,
  },
};

// ---------------------------------------------------------------------------
// Shared types (re-exported for consumers)
// ---------------------------------------------------------------------------

interface ISendMessageParams {
  input: string;
  conversation_id: ConversationId;
  files?: string[];
  idempotency_key: string;
  /** Automatic Guid/QuickStart handoff; never set for explicit user sends. */
  initial_only?: boolean;
  inject_skills?: string[];
}

// Server-assigned identifier for the newly created user message. Clients must
// use this as the canonical msg_id when rendering an optimistic bubble so the
// local state aligns with DB rows and WebSocket stream events.
export interface ISendMessageResult {
  msg_id: MessageId;
  /** The request reused an already accepted Idempotency-Key. */
  replayed: boolean;
  /** The durable receipt is terminal; this response did not open a turn. */
  completed: boolean;
  result_ok: boolean | null;
  result_text: string | null;
  result_error: string | null;
  result_error_code: string | null;
  result_error_retryable: boolean | null;
}

export interface IConfirmMessageParams {
  confirm_key: string;
  msg_id: MessageId | ConfirmationCorrelationId;
  conversation_id: ConversationId;
  call_id: string;
}

export interface ICreateConversationParams {
  type: 'nomi';
  name?: string;
  model: TProviderWithModel;
  /** Backend-resolved reusable launch configuration. */
  preset_id?: PresetReference;
  preset_overrides?: import('../types/agent/presetTypes').PresetOverrides;
  delegation_policy?: TDelegationPolicy;
  execution_model_pool?: TExecutionModelPool;
  decision_policy?: TDecisionPolicy;
  /** Optional collaboration authoring default. The first delegated Execution
   * copies the template and never retains a runtime foreign key. */
  execution_template_id?: ExecutionTemplateId;
  extra: {
    workspace?: string;
    custom_workspace?: boolean;
    default_files?: string[];
    backend?: string;
    cli_path?: string;
    gateway?: {
      host?: string;
      port?: number;
      token?: string;
      password?: string;
      use_external_gateway?: boolean;
      cli_path?: string;
    };
    web_search_engine?: 'google' | 'default';
    agent_name?: string;
    agent_id?: string;
    context?: string;
    context_file_name?: string;
    /** Transient: preset opt-in skills. Consumed by backend create handler
     *  and stripped before persistence. */
    preset_enabled_skills?: string[];
    /** Transient: auto-inject skills the user opted out of on the Guid page.
     *  Consumed by backend create handler and stripped before persistence. */
    exclude_auto_inject_skills?: string[];
    /** Transient: MCP server ids selected on the Guid page. Consumed by the
     *  backend create handler and snapshotted into conversation.extra. */
    selected_mcp_server_ids?: McpServerId[];
    /** Transient: session-scoped MCP server configs that are not stored in the
     *  backend catalog (currently built-in MCP servers). */
    selected_session_mcp_servers?: ISessionMcpServer[];
    session_mode?: string;
    codex_model?: string;
    current_model_id?: string;
    pending_config_options?: Record<string, string>;
    runtime_validation?: {
      expected_workspace?: string;
      expected_backend?: string;
      expected_agent_name?: string;
      expected_cli_path?: string;
      expected_model?: string;
      expected_identity_hash?: string | null;
      switched_at?: number;
    };
    /** Legacy marker for pre-provider-probe health-check conversations. */
    is_health_check?: boolean;
    /** Binds a nomi conversation to a saved SSH host: the remote tool family
     *  operates that host. Optional companion `ssh_remote_cwd` sets the shell's
     *  starting directory (defaults to the remote $HOME). */
    ssh_host_id?: import('../types/ids').SshHostId;
    ssh_remote_cwd?: string;
    extra_skill_paths?: string[];
  };
}

interface IResetConversationParams {
  conversation_id: ConversationId;
}

export interface IDirOrFile {
  name: string;
  fullPath: string;
  relativePath: string;
  isDir: boolean;
  isFile: boolean;
  children?: Array<IDirOrFile>;
}

export interface IFileMetadata {
  name: string;
  path: string;
  size: number;
  type: string;
  lastModified: number;
  isDirectory?: boolean;
}

export type IWorkspaceFlatFile = {
  name: string;
  fullPath: string;
  relativePath: string;
};

export interface IResponseMessage {
  type: string;
  data: unknown;
  status?: 'finish' | 'pending' | 'error' | 'work';
  /** Stable backend message UUIDv7. */
  msg_id: MessageId;
  /** Stable owning turn identity. It is distinct from msg_id for first-class
   * terminal/error rows and continuation message segments. */
  turn_id?: MessageId;
  /** For a terminal frame, the durable visible text segment that owns the
   * backend's final text rewrite. This may differ from the terminal msg_id. */
  final_text_msg_id?: MessageId;
  /** Present only when the terminal was emitted after backend final-text
   * middleware and persistence completed. Legacy terminals omit this marker. */
  final_text_authoritative?: boolean;
  /** Canonical owning conversation entity ID. */
  conversation_id: ConversationId;
  created_at?: number;
  hidden?: boolean;
  /** Replace accumulated text for the same msg_id instead of appending. */
  replace?: boolean;
  /** This content is a self-contained finalized projection, not a fragment of
   *  an active model turn. Consumers must render it without raising turn or
   *  conversation activity state. */
  stream_complete?: boolean;
  /** Companion wire markers (backend StreamRelay stamps them on every
   *  fragment): true + owning companion id when the conversation is a companion
   *  owned session. */
  companion?: boolean;
  companion_id?: CompanionId | null;
  /** IM platform ("telegram" | "lark" | ...) when the conversation is a
   *  channel-originated turn; null/absent for local conversations. */
  channel_platform?: string | null;
  /** Originating subsystem of the turn's user message (companion/cron/autowork/
   *  idmm); null/absent = typed by a real person. */
  origin?: string | null;
}

export interface IKnowledgeWritebackEvent {
  conversation_id: ConversationId;
  msg_id: MessageId;
  status:
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
  attempt_id?: string;
  attempt_generation?: number;
  started_at?: number;
  updated_at?: number;
  finished_at?: number | null;
  retryable?: boolean;
  candidates?: number;
  written?: Array<{
    kb_id?: KnowledgeBaseId | null;
    rel_path?: string | null;
  }>;
  failures?: Array<{
    kb_id?: KnowledgeBaseId | null;
    rel_path?: string | null;
    error?: string;
  }>;
}

/** `message.userCreated` broadcast: a user message was persisted (covers IM
 *  channel inbound messages — the companion window renders those as incoming
 *  bubble headers). Same companion wire markers as IResponseMessage. */
export interface IUserMessageCreatedEvent {
  conversation_id: ConversationId;
  msg_id: MessageId;
  content: string;
  position: 'right';
  status: string;
  hidden?: boolean;
  origin?: string | null;
  companion?: boolean;
  companion_id?: CompanionId | null;
  channel_platform?: string | null;
  created_at: number;
}

export type IConversationArtifactKind = 'cron_trigger' | 'skill_suggest';
export type IConversationArtifactStatus = 'active' | 'pending' | 'dismissed' | 'saved';

export interface IConversationArtifactBase<
  Kind extends IConversationArtifactKind,
  Payload extends Record<string, unknown>,
> {
  conversation_artifact_id: ConversationArtifactId;
  /** Owning canonical Conversation entity id. */
  conversation_id: ConversationId;
  /** Stable cron job business identity. */
  cron_job_id?: CronJobId;
  kind: Kind;
  status: IConversationArtifactStatus;
  payload: Payload;
  created_at: number;
  updated_at: number;
}

export type ICronTriggerArtifact = IConversationArtifactBase<
  'cron_trigger',
  {
    cron_job_id: CronJobId;
    cron_job_name: string;
    triggered_at: number;
  }
>;

export type ISkillSuggestArtifact = IConversationArtifactBase<
  'skill_suggest',
  {
    cron_job_id: CronJobId;
    name: string;
    description: string;
    skillContent?: string;
    skill_content?: string;
  }
>;

export type IConversationArtifact = ICronTriggerArtifact | ISkillSuggestArtifact;

export interface IConversationTurnStartedEvent {
  conversation_id: ConversationId;
  turn_id: MessageId;
  status: 'pending' | 'running' | 'finished';
  phase?: 'starting' | 'thinking' | 'streaming' | 'tooling' | 'waiting_permission' | string;
  state:
    | 'ai_generating'
    | 'ai_waiting_input'
    | 'ai_waiting_confirmation'
    | 'initializing'
    | 'stopped'
    | 'error'
    | 'unknown'
    | string;
  detail: string;
  can_send_message: boolean;
  runtime: {
    state: 'idle' | 'starting' | 'running' | 'waiting_confirmation';
    can_send_message: boolean;
    has_runtime: boolean;
    runtime_status?: 'pending' | 'running' | 'finished';
    is_processing: boolean;
    pending_confirmations: number;
    active_turn_id?: MessageId;
    processing_started_at?: number;
  };
  companion?: boolean;
  companion_id?: CompanionId | null;
  origin?: string | null;
  channel_platform?: string | null;
}

export interface IConversationTurnCompletedEvent {
  conversation_id: ConversationId;
  /** Stable turn correlation id. Older servers may omit it; consumers must
   * retain a runtime-state fallback for backward compatibility. */
  turn_id?: MessageId;
  status: 'pending' | 'running' | 'finished';
  state:
    | 'ai_generating'
    | 'ai_waiting_input'
    | 'ai_waiting_confirmation'
    | 'initializing'
    | 'stopped'
    | 'error'
    | 'unknown';
  detail: string;
  can_send_message: boolean;
  runtime: {
    state: 'idle' | 'starting' | 'running' | 'waiting_confirmation';
    can_send_message: boolean;
    has_runtime: boolean;
    runtime_status?: 'pending' | 'running' | 'finished';
    is_processing: boolean;
    pending_confirmations: number;
    active_turn_id?: MessageId;
    processing_started_at?: number;
  };
  workspace: string;
  model: {
    platform: string;
    name: string;
    use_model: string;
  };
  last_message: {
    message_id?: MessageId;
    type?: string;
    content: unknown;
    status?: string | null;
    created_at: number;
  };
}

export interface IConversationListChangedEvent {
  conversation_id: ConversationId;
  action: 'created' | 'updated' | 'deleted';
  source?: string;
}

export type ConversationSideQuestionResult =
  | { status: 'ok'; answer: string }
  | { status: 'noAnswer' }
  | { status: 'unsupported' }
  | { status: 'invalid'; reason: 'emptyQuestion' }
  | { status: 'toolsRequired' };

interface IBridgeResponse<D = {}> {
  success: boolean;
  data?: D;
  msg?: string;
}

// ---------------------------------------------------------------------------
// Extensions API
// ---------------------------------------------------------------------------

export interface IExtensionInfo {
  name: string;
  display_name: string;
  version: string;
  description?: string;
  source: string;
  enabled: boolean;
}

export interface IExtensionPermissionSummary {
  name: string;
  description: string;
  level: 'safe' | 'moderate' | 'dangerous';
  granted: boolean;
}

export interface IExtensionSettingsTab {
  id: string;
  label: string;
  icon?: string;
  url: string;
  position?: { relative_to: string; placement: 'before' | 'after' };
  order: number;
  extension_name: string;
}

export interface IExtensionWebuiContribution {
  extension_name: string;
  id: string;
  directory: string;
  routes: Array<{ path: string; method: string; handler: string }>;
}

export interface IExtensionMcpServerContribution {
  source_key: string;
  name: string;
  description?: string;
  enabled: boolean;
  transport: unknown;
  extension_name: string;
}

export type AgentActivityState = 'idle' | 'writing' | 'researching' | 'executing' | 'syncing' | 'error';

export interface IExtensionAgentActivityEvent {
  conversationId: ConversationId;
  at: number;
  kind: 'status' | 'tool' | 'message';
  text: string;
}

export interface IExtensionAgentActivityItem {
  id: string;
  backend: string;
  agentName: string;
  state: AgentActivityState;
  runtimeStatus: 'pending' | 'running' | 'finished' | 'unknown';
  conversations: number;
  activeConversations: number;
  lastActiveAt: number;
  lastStatus?: string;
  currentTask?: string;
  recentEvents: IExtensionAgentActivityEvent[];
}

export interface IExtensionAgentActivitySnapshot {
  generatedAt: number;
  totalConversations: number;
  runningConversations: number;
  agents: IExtensionAgentActivityItem[];
}

export const extensions = {
  getThemes: httpGet<ICssTheme[], void>('/api/extensions/themes'),
  getLoadedExtensions: httpGet<IExtensionInfo[], void>('/api/extensions'),
  getPresets: httpGet<Record<string, unknown>[], void>('/api/extensions/presets'),
  getAgents: httpGet<Record<string, unknown>[], void>('/api/extensions/agents'),
  getMcpServers: httpGet<IExtensionMcpServerContribution[], void>('/api/extensions/mcp-servers'),
  getSkills: httpGet<Array<{ name: string; description: string; location: string }>, void>('/api/extensions/skills'),
  getSettingsTabs: httpGet<IExtensionSettingsTab[], void>('/api/extensions/settings-tabs'),
  getWebuiContributions: httpGet<IExtensionWebuiContribution[], void>('/api/extensions/webui'),
  getAgentActivitySnapshot: httpGet<IExtensionAgentActivitySnapshot, void>('/api/extensions/agent-activity'),
  getExtI18nForLocale: httpPost<Record<string, unknown>, { locale: string }>('/api/extensions/i18n'),
  enableExtension: httpPost<void, { name: string }>('/api/extensions/enable'),
  disableExtension: httpPost<void, { name: string; reason?: string }>('/api/extensions/disable'),
  getPermissions: httpPost<IExtensionPermissionSummary[], { name: string }>('/api/extensions/permissions'),
  getRiskLevel: httpPost<string, { name: string }>('/api/extensions/risk-level'),
  stateChanged: wsEmitter<{ name: string; enabled: boolean; reason?: string }>('extensions.state-changed'),
};

// ---------------------------------------------------------------------------
// Channel API — routed to /api/channel/*
// ---------------------------------------------------------------------------

import type {
  ChannelOwnerDomain,
  IChannelPairingRequest,
  IChannelPluginStatus,
  IChannelSession,
  IChannelUser,
  SetGroupAccessRequest,
} from '@/common/types/channel/channel';
import { normalizeGroupAccessMode } from '@/common/types/channel/channel';

type RawPluginStatus = Record<string, unknown>;
type RawPairing = Record<string, unknown>;
type RawUser = Record<string, unknown>;
type RawSession = Record<string, unknown>;

interface IChannelBridgeResponse {
  success: boolean;
  message?: string;
  error?: string;
}

function requireSuccessfulChannelResponse(raw: IChannelBridgeResponse): void {
  if (!raw.success) {
    throw new Error(raw.error || raw.message || 'Channel operation failed');
  }
}

type IChannelEnableResponse = {
  success: boolean;
  plugin_id?: ChannelPluginId;
  error?: string;
};

function toPluginStatus(raw: RawPluginStatus): IChannelPluginStatus {
  return {
    plugin_id: parseChannelPluginId(raw.plugin_id),
    type: raw.type as string,
    name: raw.name as string,
    enabled: raw.enabled as boolean,
    connected: (raw.connected ?? false) as boolean,
    status: raw.status as string | undefined,
    last_connected: raw.last_connected as number | undefined,
    activeUsers: (raw.active_users ?? 0) as number,
    // Fail closed while talking to an older backend or receiving a future value.
    groupAccessMode: normalizeGroupAccessMode(raw.group_access_mode),
    botUsername: raw.bot_username as string | undefined,
    hasToken: (raw.has_token ?? false) as boolean,
    // 所有权分域：缺省（过渡期后端未透出）按 companion 处理，与 DB DEFAULT 一致。
    owner_domain: raw.owner_domain === 'customer_service' ? 'customer_service' : 'companion',
    companionId: raw.companion_id == null ? undefined : parseCompanionId(raw.companion_id),
    botKey: raw.bot_key as string | undefined,
    isExtension: raw.is_extension as boolean | undefined,
    extensionMeta: raw.extension_meta as IChannelPluginStatus['extensionMeta'],
  };
}

function toPairing(raw: RawPairing): IChannelPairingRequest {
  return {
    code: raw.code as string,
    platformUserId: raw.platform_user_id as string,
    platformType: raw.platform_type as string,
    display_name: raw.display_name as string | undefined,
    requestedAt: raw.requested_at as number,
    expiresAt: raw.expires_at as number,
    channel_plugin_id:
      raw.channel_plugin_id == null ? undefined : parseChannelPluginId(raw.channel_plugin_id),
  };
}

function toChannelUser(raw: RawUser): IChannelUser {
  return {
    channel_user_id: parseChannelUserId(raw.channel_user_id),
    platformUserId: raw.platform_user_id as string,
    platformType: raw.platform_type as string,
    display_name: raw.display_name as string | undefined,
    authorizedAt: raw.authorized_at as number,
    lastActive: raw.last_active as number | undefined,
    channel_session_id:
      raw.channel_session_id == null ? undefined : parseChannelSessionId(raw.channel_session_id),
    channel_plugin_id:
      raw.channel_plugin_id == null ? undefined : parseChannelPluginId(raw.channel_plugin_id),
  };
}

function toChannelSession(raw: RawSession): IChannelSession {
  return {
    channel_session_id: parseChannelSessionId(raw.channel_session_id),
    channel_user_id: parseChannelUserId(raw.channel_user_id),
    agent_type: raw.agent_type as string,
    conversation_id: raw.conversation_id == null ? undefined : parseConversationId(raw.conversation_id),
    workspace: raw.workspace as string | undefined,
    chatId: raw.chat_id as string | undefined,
    channel_plugin_id:
      raw.channel_plugin_id == null ? undefined : parseChannelPluginId(raw.channel_plugin_id),
    created_at: raw.created_at as number,
    lastActivity: raw.last_activity as number,
  };
}

export const channel = {
  getPluginStatus: withResponseMap(httpGet<RawPluginStatus[], void>('/api/channel/plugins'), (raw) =>
    raw.map(toPluginStatus)
  ),
  /**
   * 启用/更新机器人渠道。寻址契约（对应后端 EnableChannelSpec）：
   * - 裸 UUIDv7 `plugin_id` 指向已有渠道实体 → 更新该实体；
   * - 省略 `plugin_id` 并给 `plugin_type` → 新建一行（每宠多机器人路径）；
   * - `companion_id` 把机器人绑到桌面伙伴；同一机器人(bot_key)已绑其他对象时后端 409。
   *   （客服绑定归客服域所有：PUT /api/customer-service/agents/{id}/bindings。）
   * - `owner_domain` 仅创建时可选（缺省 companion）；'customer_service' 域的行
   *   与 companion_id 互斥（后端 400/ABORT）。
   */
  enablePlugin: withResponseMap(httpPost<
    { success: boolean; plugin_id?: unknown; error?: string },
    {
      plugin_id?: import('../types/ids').ChannelPluginId;
      plugin_type?: string;
      companion_id?: CompanionId;
      owner_domain?: ChannelOwnerDomain;
      config: Record<string, unknown>;
    }
  >('/api/channel/plugins/enable'), (raw): IChannelEnableResponse => {
    return {
      success: raw.success,
      ...(raw.plugin_id == null ? {} : { plugin_id: parseChannelPluginId(raw.plugin_id) }),
      ...(raw.error == null ? {} : { error: raw.error }),
    };
  }),
  disablePlugin: withResponseMap(
    httpPost<IChannelBridgeResponse, { plugin_id: import('../types/ids').ChannelPluginId }>(
      '/api/channel/plugins/disable'
    ),
    requireSuccessfulChannelResponse
  ),
  /** 删除渠道行：停实例 + 清该渠道会话 + 删行（会话所产生的对话保留）。 */
  deletePlugin: withResponseMap(
    httpPost<IChannelBridgeResponse, { plugin_id: import('../types/ids').ChannelPluginId }>(
      '/api/channel/plugins/delete'
    ),
    requireSuccessfulChannelResponse
  ),
  testPlugin: httpPost<
    { success: boolean; bot_username?: string; error?: string },
    { plugin_type: string; token: string; extra_config?: { app_id?: string; app_secret?: string; app_token?: string; homeserver_url?: string; user_id?: string; server_url?: string; nostr_relays?: string } }
  >('/api/channel/plugins/test'),
  getPendingPairings: withResponseMap(httpGet<RawPairing[], void>('/api/channel/pairings'), (raw) =>
    raw.map(toPairing)
  ),
  approvePairing: httpPost<void, { code: string }>('/api/channel/pairings/approve'),
  rejectPairing: httpPost<void, { code: string }>('/api/channel/pairings/reject'),
  getAuthorizedUsers: withResponseMap(httpGet<RawUser[], void>('/api/channel/users'), (raw) => raw.map(toChannelUser)),
  revokeUser: httpPost<void, { channel_user_id: import('../types/ids').ChannelUserId }>(
    '/api/channel/users/revoke'
  ),
  /** Update one bot row's group-chat policy; direct-message pairing is unchanged. */
  setGroupAccess: httpPost<void, SetGroupAccessRequest>('/api/channel/settings/group-access'),
  getActiveSessions: withResponseMap(httpGet<RawSession[], void>('/api/channel/sessions'), (raw) =>
    raw.map(toChannelSession)
  ),
  syncChannelSettings: httpPost<void, { platform: string }>('/api/channel/settings/sync'),
  /**
   * Bind one companion to an IM channel platform.
   * Atomic on the backend: writes the channel companion preference and resets
   * the platform's active sessions in one step.
   * Omitted/null `companion_id` clears the binding; empty strings are invalid.
   * Binding a non-existent companion returns 400 — errors propagate to the caller
   * as `BackendHttpError`.
   */
  setChannelCompanion: httpPost<
    void,
    {
      platform?: string;
      plugin_id?: import('../types/ids').ChannelPluginId;
      companion_id?: CompanionId | null;
    }
  >(
    '/api/channel/settings/companion'
  ),
  /**
   * 启动微信扫码登录流程。后端立即返回，二维码生命周期事件经 WebSocket 的
   * `weixinLogin` 推送。改用 WS（不再用 SSE）：`EventSource` 带不了桌面的
   * `x-nomi-local-trust` 头，旧 SSE 流被鉴权中间件 403 → 前端秒弹"微信登录失败"。
   */
  startWeixinLogin: httpPost<void, void>('/api/channel/weixin/login/start'),
  pairingRequested: wsMappedEmitter<IChannelPairingRequest, unknown>('channel.pairing-requested', (raw) =>
    toPairing(raw as RawPairing)
  ),
  pluginStatusChanged: wsMappedEmitter<{
    plugin_id: import('../types/ids').ChannelPluginId;
    status: IChannelPluginStatus;
  }>('channel.plugin-status-changed', (raw) => {
    const r = raw as Record<string, unknown>;
    return {
      plugin_id: parseChannelPluginId(r.plugin_id),
      status: toPluginStatus(r.status as RawPluginStatus),
    };
  }),
  userAuthorized: wsMappedEmitter<IChannelUser, unknown>('channel.user-authorized', (raw) => toChannelUser(raw as RawUser)),
  /** Channel events are not replayed; reload durable pairings/users after a
   * successful WebSocket reconnect to cover the disconnected interval. */
  reconnected: wsEmitter<undefined>('ws.reconnected'),
  /**
   * 微信扫码登录生命周期事件（替代旧 SSE 流）。`phase` 区分阶段：
   * `qr`(带 qrcodeData) → `scanned` → 终态 `done`(带 accountId/botToken) 或 `error`(带 message)。
   */
  weixinLogin: wsEmitter<{
    phase: 'qr' | 'scanned' | 'done' | 'error';
    qrcodeData?: string;
    accountId?: string;
    botToken?: string;
    baseUrl?: string;
    message?: string;
  }>('channel.weixin-login'),
};

// ---------------------------------------------------------------------------
// Agent Hub API — routed to /api/hub/*
// ---------------------------------------------------------------------------

import type { HubExtensionStatus, IHubAgentItem } from '@/common/types/agent/hub';
import type { AgentMetadata } from '@/renderer/utils/model/agentTypes';

export const hub = {
  getExtensionList: httpGet<IHubAgentItem[], void>('/api/hub/extensions'),
  install: httpPost<void, { name: string }>('/api/hub/install'),
  uninstall: httpPost<void, { name: string }>('/api/hub/uninstall'),
  retryInstall: httpPost<void, { name: string }>('/api/hub/retry-install'),
  checkUpdates: httpPost<{ name: string }[], void>('/api/hub/check-updates'),
  update: httpPost<void, { name: string }>('/api/hub/update'),
  onStateChanged: wsEmitter<{
    name: string;
    status: HubExtensionStatus;
    error?: string;
  }>('hub.state-changed'),
};

// ── Requirements Platform (需求平台) ─────────────────────────────────

export type RequirementStatus = 'pending' | 'in_progress' | 'done' | 'failed' | 'cancelled' | 'needs_review';

export interface IAttachment {
  id: AttachmentId;
  file_name: string;
  mime: string;
  size_bytes: number;
  created_at: number;
  /** Absolute path resolved by the backend at read time, for image-base64 display. */
  abs_path: string;
}

/** Raw attachment shape returned by the backend.
 * `attachment_id` is the stable UUIDv7 business identity; SQLite row ids
 * never cross this API boundary.
 */
export interface AttachmentResponse {
  attachment_id: string;
  file_name: string;
  mime: string;
  size_bytes: number;
  created_at: number;
  abs_path: string;
}

export interface INewAttachmentRef {
  /** Absolute path returned by POST /api/fs/upload (must be inside the temp upload root). */
  source_path: string;
  file_name: string;
}

export interface IRequirement {
  /** Stable bare UUIDv7 business identity. SQLite technical row ids never cross this boundary. */
  requirement_id: RequirementId;
  /** Compact, immutable human-facing identifier, rendered as `#N`. */
  display_no: number;
  title: string;
  content: string;
  tag: string;
  order_key: string;
  status: RequirementStatus;
  completion_note?: string;
  owner_conversation_id?: ConversationId;
  owner_terminal_id?: TerminalId;
  started_at?: number;
  completed_at?: number;
  attempt_count: number;
  created_by: string;
  created_at: number;
  updated_at: number;
  /** Only present on get/create/update responses — list/board rows omit attachments to keep payloads small. */
  attachments?: IAttachment[];
}

type RequirementResponse = Omit<
  IRequirement,
  'requirement_id' | 'owner_conversation_id' | 'owner_terminal_id' | 'attachments'
> & {
  requirement_id: unknown;
  owner_conversation_id?: unknown;
  owner_terminal_id?: unknown;
  attachments?: AttachmentResponse[];
};

/** Whitelisted sort columns for the requirements list (server validates too). */
export type RequirementOrderBy =
  | 'display_no'
  | 'requirement_id'
  | 'created_at'
  | 'updated_at'
  | 'status';

export interface IListRequirementsParams {
  tag?: string;
  status?: RequirementStatus;
  /** Filter by owning conversation id. */
  conversation_id?: ConversationId;
  q?: string;
  /** Sort column; omit for the default queue order (sort_seq, priority, created_at). */
  order_by?: RequirementOrderBy;
  /** Sort direction; server defaults to 'desc' when order_by is set. */
  order?: 'asc' | 'desc';
  page?: number;
  page_size?: number;
}

export interface ICreateRequirementParams {
  title: string;
  content?: string;
  tag: string;
  order_key?: string;
  status?: RequirementStatus;
  created_by?: string;
  attachments?: INewAttachmentRef[];
}

export interface IUpdateRequirementParams {
  title?: string;
  content?: string;
  tag?: string;
  order_key?: string;
  status?: RequirementStatus;
  completion_note?: string;
  add_attachments?: INewAttachmentRef[];
  remove_attachment_ids?: AttachmentId[];
}

export interface ITagSummary {
  tag: string;
  pending: number;
  in_progress: number;
  done: number;
  failed: number;
  cancelled: number;
  needs_review: number;
  total: number;
  /** AutoWork is paused for this tag (a requirement exhausted its retries).
   * While true, automatic execution does not claim this tag's requirements until
   * the tag is resumed. */
  paused: boolean;
  /** Why the tag was paused (`requirement_failed` | `manual` | …). */
  paused_reason?: string;
}

export interface IBoardResponse {
  tag: string;
  pending: IRequirement[];
  in_progress: IRequirement[];
  done: IRequirement[];
  failed: IRequirement[];
  cancelled: IRequirement[];
  needs_review: IRequirement[];
}

type BoardResponse = Omit<
  IBoardResponse,
  'pending' | 'in_progress' | 'done' | 'failed' | 'cancelled' | 'needs_review'
> & {
  pending: RequirementResponse[];
  in_progress: RequirementResponse[];
  done: RequirementResponse[];
  failed: RequirementResponse[];
  cancelled: RequirementResponse[];
  needs_review: RequirementResponse[];
};

/** Broadcast (`autowork.tagPaused`) when AutoWork pauses a tag because one of
 * its requirements exhausted its retries. */
export interface ITagPausedPayload {
  tag: string;
  reason: string;
  requirement_id?: RequirementId;
}

export type AutoWorkTargetKind = 'conversation' | 'terminal';
export type AutoWorkRunState = 'off' | 'idle' | 'active';
export type SessionCapabilityTargetId = ConversationId | TerminalId;

export interface IAutoWorkConfigParams {
  kind: AutoWorkTargetKind;
  target_id: SessionCapabilityTargetId;
  enabled: boolean;
  tag?: string;
  max_requirements?: number;
  /** Set by the AutoWork admin (标签会话管理). When true, the backend rejects
   * disabling an actively-executing session — the user must stop it from the
   * session page. Session-page toggles leave this unset. */
  from_admin?: boolean;
}

export interface IAutoWorkState {
  kind: AutoWorkTargetKind;
  target_id: SessionCapabilityTargetId;
  enabled: boolean;
  tag?: string;
  running: boolean;
  run_state: AutoWorkRunState;
  current_requirement_id?: RequirementId;
  completed_count: number;
}

export const fromAttachmentResponse = (attachment: AttachmentResponse): IAttachment => {
  const { attachment_id, ...fields } = attachment;
  return {
    ...fields,
    id: parseAttachmentId(attachment_id),
  };
};

const fromApiRequirement = (requirement: RequirementResponse): IRequirement => {
  const {
    requirement_id,
    owner_conversation_id,
    owner_terminal_id,
    attachments,
    ...fields
  } = requirement;
  return {
    ...fields,
    requirement_id: parseRequirementId(requirement_id),
    ...(owner_conversation_id == null
      ? {}
      : { owner_conversation_id: parseConversationId(owner_conversation_id) }),
    ...(owner_terminal_id == null
      ? {}
      : { owner_terminal_id: parseTerminalId(owner_terminal_id) }),
    ...(attachments == null
      ? {}
      : { attachments: attachments.map(fromAttachmentResponse) }),
  };
};

const fromApiAutoWorkState = (state: IAutoWorkState): IAutoWorkState => ({
  ...state,
  target_id: state.kind === 'conversation'
    ? parseConversationId(state.target_id)
    : parseTerminalId(state.target_id),
  ...(state.current_requirement_id != null
    ? { current_requirement_id: parseRequirementId(state.current_requirement_id) }
    : {}),
});

export const requirements = {
  list: withResponseMap(httpGet<PaginatedResult<RequirementResponse>, IListRequirementsParams>((p) => {
    const q = new URLSearchParams();
    if (p?.tag) q.set('tag', p.tag);
    if (p?.status) q.set('status', p.status);
    if (p?.conversation_id != null) q.set('conversation_id', p.conversation_id);
    if (p?.q) q.set('q', p.q);
    if (p?.order_by) q.set('order_by', p.order_by);
    if (p?.order) q.set('order', p.order);
    if (p?.page != null) q.set('page', String(p.page));
    if (p?.page_size != null) q.set('page_size', String(p.page_size));
    const qs = q.toString();
    return `/api/requirements${qs ? `?${qs}` : ''}`;
  }), (page) => ({ ...page, items: page.items.map(fromApiRequirement) })),
  get: withResponseMap(httpGet<RequirementResponse, { requirement_id: RequirementId }>((p) => `/api/requirements/${p.requirement_id}`), fromApiRequirement),
  create: withResponseMap(httpPost<RequirementResponse, ICreateRequirementParams>('/api/requirements'), fromApiRequirement),
  update: withResponseMap(httpPut<RequirementResponse, { requirement_id: RequirementId; updates: IUpdateRequirementParams }>(
    (p) => `/api/requirements/${p.requirement_id}`,
    (p) => p.updates
  ), fromApiRequirement),
  remove: httpDelete<void, { requirement_id: RequirementId }>((p) => `/api/requirements/${p.requirement_id}`),
  batchDelete: httpPost<{ deleted: number }, { requirement_ids: RequirementId[] }>('/api/requirements/batch-delete'),
  tags: httpGet<ITagSummary[], void>('/api/requirements/tags'),
  board: withResponseMap(httpGet<BoardResponse, { tag: string }>((p) => `/api/requirements/board?tag=${encodeURIComponent(p.tag)}`), (board): IBoardResponse => ({
    ...board,
    pending: board.pending.map(fromApiRequirement),
    in_progress: board.in_progress.map(fromApiRequirement),
    done: board.done.map(fromApiRequirement),
    failed: board.failed.map(fromApiRequirement),
    cancelled: board.cancelled.map(fromApiRequirement),
    needs_review: board.needs_review.map(fromApiRequirement),
  })),
  setAutoWork: withResponseMap(httpPost<IAutoWorkState, IAutoWorkConfigParams>('/api/requirements/autowork'), fromApiAutoWorkState),
  getAutoWork: withResponseMap(httpGet<IAutoWorkState, { kind: AutoWorkTargetKind; target_id: SessionCapabilityTargetId }>(
    (p) => `/api/requirements/autowork/${p.kind}/${p.target_id}`
  ), fromApiAutoWorkState),
  resumeTag: httpPost<ITagSummary, { tag: string; requeue_failed?: boolean; requeue_requirement_ids?: RequirementId[] }>(
    (p) => `/api/requirements/tags/${encodeURIComponent(p.tag)}/resume`,
    (p) => ({
      requeue_failed: p.requeue_failed,
      requeue_requirement_ids: p.requeue_requirement_ids,
    })
  ),
  onCreated: wsMappedEmitter<IRequirement, RequirementResponse>('requirement.created', fromApiRequirement),
  onUpdated: wsMappedEmitter<IRequirement, RequirementResponse>('requirement.updated', fromApiRequirement),
  onStatusChanged: wsMappedEmitter<IRequirement, RequirementResponse>('requirement.statusChanged', fromApiRequirement),
  onDeleted: wsMappedEmitter<{ requirement_id: RequirementId }>('requirement.deleted', (value) => ({
    requirement_id: parseRequirementId(value.requirement_id),
  })),
  onAutoWork: wsMappedEmitter<IAutoWorkState>('autowork.statusChanged', fromApiAutoWorkState),
  onTagPaused: wsMappedEmitter<ITagPausedPayload>('autowork.tagPaused', (value) => ({
    ...value,
    ...(value.requirement_id != null ? { requirement_id: parseRequirementId(value.requirement_id) } : {}),
  })),
  tagBindings: withResponseMap(httpGet<ITagBindings[], void>('/api/requirements/tag-bindings'), (groups) =>
    groups.map((group) => ({
      ...group,
      bindings: group.bindings.map((binding) => ({
        ...binding,
        target_id: binding.kind === 'conversation'
          ? parseConversationId(binding.target_id)
          : parseTerminalId(binding.target_id),
      })),
    }))
  ),
};

// ─────────────────────────── IDMM (Intelligent Decision-Making Mode) ───────────────────────────

export type IdmmTargetKind = 'conversation' | 'terminal';
export type IdmmRunState = 'off' | 'armed' | 'intervening';

// ── Phase-2 dual-watch config (mirrors `nomifun-api-types/src/idmm.rs` D1/D2). ──
// IDMM is reorganized into two independently-toggleable, default-off watches that
// share one engine: 故障值守 (fault watch) and 决策值守 (decision watch). The
// backend flattens `WatchBase` into each watch (serde `#[flatten]`), so the base
// knobs live at the top level of each watch object on the wire.

/** Rule-only (no model) vs rule + backup model. */
export type IdmmWatchTier = 'rule_only' | 'rule_plus_model';

/** How much context the watch scans / feeds the backup model. */
export type IdmmScanScope = 'last_turn' | 'last_messages' | 'full_session';

/** Backup ("bypass") model the watch escalates to (empty → global default → session model). */
export interface IIdmmBypassModelRef {
  provider_id?: ProviderId | null;
  model?: string | null;
}

/** Rate limits to keep a watch from thrashing a session. */
export interface IIdmmBudgetConfig {
  max_interventions_per_hour: number;
  min_interval_secs: number;
}

/** Shared base knobs flattened into each watch config. */
export interface IIdmmWatchBase {
  enabled: boolean;
  tier: IdmmWatchTier;
  /** 监测间隔 (was idle_threshold_secs). */
  scan_interval_secs: number;
  /** 最大重试. */
  max_retries: number;
  /** 扫描范围. */
  scan_scope: IdmmScanScope;
  /** Context-char ceiling fed to the bypass model (carried over default 8000). */
  max_context_chars: number;
  /** 旁路模型. */
  bypass_model: IIdmmBypassModelRef;
  budget: IIdmmBudgetConfig;
}

/** P3 fault failover strategy; P2 only Retry is live. */
export type IdmmWakeStrategy = 'retry' | 'failover' | 'failover_then_retry';

/** 故障值守 — base flattened to top level + fault-specific fields. */
export interface IIdmmFaultWatchConfig extends IIdmmWatchBase {
  wake_action: IdmmWakeStrategy;
  use_failover_queue: boolean;
}

// ── Decision strategy (D2) ──

export type IdmmTendency = 'conservative' | 'balanced' | 'aggressive';
export type IdmmBlockedBehavior = 'prefer_continue' | 'prefer_pause' | 'must_ask';
export type IdmmCategoryMode = 'auto' | 'ask_first' | 'off';

export interface IIdmmOptionRule {
  mode: IdmmCategoryMode;
  prefer_recommended: boolean;
  allow_unmarked_pick: boolean;
  never_destructive: boolean;
}
export interface IIdmmOpenQuestionRule {
  mode: IdmmCategoryMode;
  max_answer_chars: number;
}
export interface IIdmmPermissionRule {
  mode: IdmmCategoryMode;
  only_safe_value: boolean;
  escalate_risky: boolean;
}
export interface IIdmmCategoryRules {
  option_decision: IIdmmOptionRule;
  open_question: IIdmmOpenQuestionRule;
  permission: IIdmmPermissionRule;
}
export interface IIdmmDecisionStrategy {
  tendency: IdmmTendency;
  on_blocked: IdmmBlockedBehavior;
  categories: IIdmmCategoryRules;
  /** 自由文本策略 — appended to the bypass-model prompt (model tier only). */
  freeform_policy?: string | null;
}

/** 决策值守 — base flattened to top level + decision-specific fields. */
export interface IIdmmDecisionWatchConfig extends IIdmmWatchBase {
  strategy: IIdmmDecisionStrategy;
  /** 纯问答开关 — answer open-ended questions (only effective at rule_plus_model). */
  answer_open_questions: boolean;
}

export interface IIdmmConfig {
  fault_watch: IIdmmFaultWatchConfig;
  decision_watch: IIdmmDecisionWatchConfig;
}

/** POST /api/idmm body: kind + target_id + a (flattened) IdmmConfig. */
export interface IIdmmSetParams extends IIdmmConfig {
  kind: IdmmTargetKind;
  target_id: SessionCapabilityTargetId;
}

export interface IIdmmState {
  kind: IdmmTargetKind;
  target_id: SessionCapabilityTargetId;
  /** True when either watch is enabled. */
  enabled: boolean;
  run_state: IdmmRunState;
  interventions_count: number;
  last_signal?: string;
  last_intervention_at?: number;
  /** Whether a backup provider is resolvable (per-session or global default). */
  sidecar_provider_resolved: boolean;
  /**
   * Persisted per-session IdmmConfig — the form's source of truth on remount.
   * Absent for targets that have never been saved. Without this round-trip,
   * user edits would silently disappear after navigation.
   */
  config?: IIdmmConfig;
}

/** One persisted IDMM decision (the "思路"/audit trail row). Field names mirror
 * the backend `InterventionRecord` JSON exactly. `target_id` is polymorphic on
 * the wire (conversation/terminal id serialized as a string). */
export interface IIdmmIntervention {
  intervention_id: IdmmInterventionId;
  target_kind: IdmmTargetKind;
  target_id: SessionCapabilityTargetId;
  /** Which watch fired: 'fault' | 'decision'. */
  watch: string;
  at: number;
  stall_class: string;
  tier_used: string;
  /** option / open_question / permission / fault. */
  category?: string;
  action: string;
  /** What was picked/answered (truncated server-side). */
  detail?: string;
  outcome: string;
  /** The reasoning ("思路") — model reason or a rule explanation. */
  reason?: string;
  /** Model confidence (null for the rule tier). */
  confidence?: number;
  /** provider/model used (null for the rule tier). */
  bypass_model?: string;
}

const parseIdmmTargetId = (kind: IdmmTargetKind, value: unknown): SessionCapabilityTargetId =>
  kind === 'conversation' ? parseConversationId(value) : parseTerminalId(value);

const fromApiIdmmConfig = (config: IIdmmConfig): IIdmmConfig => ({
  ...config,
  fault_watch: {
    ...config.fault_watch,
    bypass_model: {
      ...config.fault_watch.bypass_model,
      provider_id: config.fault_watch.bypass_model.provider_id == null
        ? config.fault_watch.bypass_model.provider_id
        : parseProviderId(config.fault_watch.bypass_model.provider_id),
    },
  },
  decision_watch: {
    ...config.decision_watch,
    bypass_model: {
      ...config.decision_watch.bypass_model,
      provider_id: config.decision_watch.bypass_model.provider_id == null
        ? config.decision_watch.bypass_model.provider_id
        : parseProviderId(config.decision_watch.bypass_model.provider_id),
    },
  },
});

const fromApiIdmmState = (state: IIdmmState): IIdmmState => ({
  ...state,
  target_id: parseIdmmTargetId(state.kind, state.target_id),
  ...(state.config ? { config: fromApiIdmmConfig(state.config) } : {}),
});

const fromApiIdmmIntervention = (record: IIdmmIntervention): IIdmmIntervention => ({
  ...record,
  intervention_id: parseIdmmInterventionId(record.intervention_id),
  target_id: parseIdmmTargetId(record.target_kind, record.target_id),
});

export const idmm = {
  set: withResponseMap(httpPost<IIdmmState, IIdmmSetParams>('/api/idmm'), fromApiIdmmState),
  getStatus: withResponseMap(httpGet<IIdmmState, { kind: IdmmTargetKind; target_id: SessionCapabilityTargetId }>(
    (p) => `/api/idmm/${p.kind}/${p.target_id}`
  ), fromApiIdmmState),
  intervene: withResponseMap(httpPost<IIdmmState, { kind: IdmmTargetKind; target_id: SessionCapabilityTargetId }>(
    (p) => `/api/idmm/${p.kind}/${p.target_id}/intervene`,
    () => ({})
  ), fromApiIdmmState),
  getLog: withResponseMap(httpGet<IIdmmIntervention[], { kind: IdmmTargetKind; target_id: SessionCapabilityTargetId; limit?: number }>(
    (p) => `/api/idmm/${p.kind}/${p.target_id}/log${p.limit ? `?limit=${p.limit}` : ''}`
  ), (records) => records.map(fromApiIdmmIntervention)),
  clearLog: httpDelete<void, { kind: IdmmTargetKind; target_id: SessionCapabilityTargetId }>(
    (p) => `/api/idmm/${p.kind}/${p.target_id}/log`
  ),
  onStatus: wsMappedEmitter<IIdmmState>('idmm.statusChanged', fromApiIdmmState),
  onIntervention: wsMappedEmitter<IIdmmIntervention>('idmm.intervention', fromApiIdmmIntervention),
};

// ── Phase-3 model failover queue (mirrors `ModelFailoverConfig`, plan D1/D8). ──
// A global, ordered list of provider+model candidates the conversation send-loop
// falls back through when a NOMI session hits a pre-response provider fault. Read
// & written through the `agent.model_failover` client preference (one JSON blob).

/** One ordered candidate in the failover queue. */
export interface IModelFailoverCandidate {
  provider_id: ProviderId;
  model: string;
}

/** Global model-failover config persisted under `agent.model_failover`. */
export interface IModelFailoverConfig {
  /** Master switch; default false. */
  enabled: boolean;
  /** Ordered candidates tried head-to-tail on a pre-response provider fault. */
  queue: IModelFailoverCandidate[];
  /** Per-turn cap on switches (also bounded by `queue.length`); default 4. */
  max_switches: number;
}

const fromApiModelFailoverConfig = (config: IModelFailoverConfig): IModelFailoverConfig => ({
  enabled: config.enabled,
  max_switches: config.max_switches,
  queue: config.queue.map((candidate) => ({
    model: candidate.model,
    provider_id: parseProviderId(candidate.provider_id),
  })),
});

export const agentModelFailover = {
  getSettings: withResponseMap(
    httpGet<IModelFailoverConfig, void>('/api/agent/model-failover'),
    fromApiModelFailoverConfig
  ),
  updateSettings: withResponseMap(
    httpPut<IModelFailoverConfig, IModelFailoverConfig>('/api/agent/model-failover'),
    fromApiModelFailoverConfig
  ),
};

// ─────────────────────────── Webhook + AutoWork admin ───────────────────────────

/** AutoWork tag→session binding (a session whose AutoWork is enabled on a tag). */
export interface ITagBinding {
  kind: AutoWorkTargetKind;
  target_id: SessionCapabilityTargetId;
  name: string;
  run_state: AutoWorkRunState;
}

/** All AutoWork bindings for one tag (used by 标签会话管理). */
export interface ITagBindings {
  tag: string;
  bindings: ITagBinding[];
}

export type IWebhookPlatform = 'lark' | 'http' | 'slack';

/** A webhook endpoint. The signing `secret` is never returned — `has_secret`
 * signals whether one is stored. */
export interface IWebhook {
  webhook_id: WebhookId;
  name: string;
  platform: IWebhookPlatform;
  url: string;
  description: string;
  has_secret: boolean;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

export interface ICreateWebhookParams {
  name: string;
  url: string;
  platform?: IWebhookPlatform;
  description?: string;
  /** Optional Lark signing secret (加签). */
  secret?: string | null;
  enabled?: boolean;
}

/** Partial update. `secret`: omit = keep, `null` = clear, string = set. */
export interface IUpdateWebhookParams {
  name?: string;
  url?: string;
  platform?: IWebhookPlatform;
  description?: string;
  secret?: string | null;
  enabled?: boolean;
}

/** Per-tag settings (bound webhook + description) over the implicit tags. */
export interface ITagSetting {
  tag: string;
  webhook_id?: WebhookId | null;
  description: string;
  /** Event kinds that trigger a notification for this tag. */
  notify_events: string[];
}

export interface IUpsertTagSettingParams {
  /** omit = keep, `null` = clear, canonical UUIDv7 webhook ID = bind. */
  webhook_id?: WebhookId | null;
  description?: string;
  /** omit = keep, array = replace the notify-event set. */
  notify_events?: string[];
}

const fromApiWebhook = (value: IWebhook): IWebhook => ({
  ...value,
  webhook_id: parseWebhookId(value.webhook_id),
});

const fromApiTagSetting = (value: ITagSetting): ITagSetting => ({
  ...value,
  ...(value.webhook_id != null ? { webhook_id: parseWebhookId(value.webhook_id) } : {}),
});

export const webhook = {
  list: withResponseMap(httpGet<IWebhook[], void>('/api/webhooks'), (items) => items.map(fromApiWebhook)),
  get: withResponseMap(httpGet<IWebhook, { webhook_id: WebhookId }>((p) => `/api/webhooks/${p.webhook_id}`), fromApiWebhook),
  create: withResponseMap(httpPost<IWebhook, ICreateWebhookParams>('/api/webhooks'), fromApiWebhook),
  update: withResponseMap(httpPut<IWebhook, { webhook_id: WebhookId; updates: IUpdateWebhookParams }>(
    (p) => `/api/webhooks/${p.webhook_id}`,
    (p) => p.updates
  ), fromApiWebhook),
  remove: httpDelete<void, { webhook_id: WebhookId }>((p) => `/api/webhooks/${p.webhook_id}`),
  test: httpPost<void, { webhook_id: WebhookId }>(
    (p) => `/api/webhooks/${p.webhook_id}/test`,
    () => ({})
  ),
  getTagSetting: withResponseMap(httpGet<ITagSetting, { tag: string }>((p) => `/api/tags/${encodeURIComponent(p.tag)}/settings`), fromApiTagSetting),
  setTagSetting: withResponseMap(httpPut<ITagSetting, { tag: string; updates: IUpsertTagSettingParams }>(
    (p) => `/api/tags/${encodeURIComponent(p.tag)}/settings`,
    (p) => p.updates
  ), fromApiTagSetting),
};

// Persistent Agent Execution is the sole collaboration transport exposed to the
// renderer. Planning, routing, scheduling and retries remain implementation
// details behind this aggregate.
const executionWireObject = (raw: unknown): Record<string, unknown> => {
  if (raw === null || typeof raw !== 'object' || Array.isArray(raw)) {
    throw new TypeError('agent execution payload must be a JSON object');
  }
  return raw as Record<string, unknown>;
};

const fromApiAgentExecution = (raw: unknown): TAgentExecution => {
  const value = executionWireObject(raw);
  return {
    ...(value as unknown as TAgentExecution),
    execution_id: parseExecutionId(value.execution_id),
    lead_conversation_id: value.lead_conversation_id == null ? null : parseConversationId(value.lead_conversation_id),
  };
};

const fromApiExecutionParticipant = (raw: unknown): TExecutionParticipant => {
  const value = executionWireObject(raw);
  return {
    ...(value as unknown as TExecutionParticipant),
    participant_id: parseExecutionParticipantId(value.participant_id),
    execution_id: parseExecutionId(value.execution_id),
    source_agent_id: parseAgentId(value.source_agent_id),
    preset_id: value.preset_id as TExecutionParticipant['preset_id'],
    provider_id: value.provider_id == null ? null : parseProviderId(value.provider_id),
  };
};

const fromApiExecutionStep = (raw: unknown): TExecutionStep => {
  const value = executionWireObject(raw);
  return {
    ...(value as unknown as TExecutionStep),
    step_id: parseExecutionStepId(value.step_id),
    execution_id: parseExecutionId(value.execution_id),
    assigned_participant_id:
      value.assigned_participant_id == null ? null : parseExecutionParticipantId(value.assigned_participant_id),
  };
};

const fromApiExecutionDependency = (raw: unknown): TExecutionStepDependency => {
  const value = executionWireObject(raw);
  return {
    ...(value as unknown as TExecutionStepDependency),
    execution_id: parseExecutionId(value.execution_id),
    blocker_step_id: parseExecutionStepId(value.blocker_step_id),
    blocked_step_id: parseExecutionStepId(value.blocked_step_id),
  };
};

const fromApiExecutionAttempt = (raw: unknown): TExecutionAttempt => {
  const value = executionWireObject(raw);
  return {
    ...(value as unknown as TExecutionAttempt),
    attempt_id: parseExecutionAttemptId(value.attempt_id),
    execution_id: parseExecutionId(value.execution_id),
    step_id: parseExecutionStepId(value.step_id),
    participant_id: value.participant_id == null ? null : parseExecutionParticipantId(value.participant_id),
    conversation_id: value.conversation_id == null ? null : parseConversationId(value.conversation_id),
  };
};

const fromApiAgentExecutionDetail = (raw: unknown): TAgentExecutionDetail => {
  const value = executionWireObject(raw);
  return {
    execution: fromApiAgentExecution(value.execution),
    participants: (value.participants as unknown[]).map(fromApiExecutionParticipant),
    steps: (value.steps as unknown[]).map(fromApiExecutionStep),
    dependencies: (value.dependencies as unknown[]).map(fromApiExecutionDependency),
    attempts: (value.attempts as unknown[]).map(fromApiExecutionAttempt),
  };
};

const fromApiAgentExecutionEvent = (raw: unknown): TAgentExecutionEvent => {
  const value = executionWireObject(raw);
  return {
    ...(value as unknown as TAgentExecutionEvent),
    execution_id: parseExecutionId(value.execution_id),
    step_id: value.step_id == null ? null : parseExecutionStepId(value.step_id),
    attempt_id: value.attempt_id == null ? null : parseExecutionAttemptId(value.attempt_id),
    actor_conversation_id:
      value.actor_conversation_id == null ? null : parseConversationId(value.actor_conversation_id),
    actor_attempt_id: value.actor_attempt_id == null ? null : parseExecutionAttemptId(value.actor_attempt_id),
    on_behalf_of_user_id: parseUserId(value.on_behalf_of_user_id),
  };
};

const fromApiExecutionTemplate = (raw: unknown): TAgentExecutionTemplate => {
  const value = executionWireObject(raw);
  return {
    ...(value as unknown as TAgentExecutionTemplate),
    execution_template_id: parseExecutionTemplateId(value.execution_template_id),
  };
};

const fromApiExecutionTemplateParticipant = (raw: unknown): TAgentExecutionTemplateParticipant => {
  const value = executionWireObject(raw);
  return {
    ...(value as unknown as TAgentExecutionTemplateParticipant),
    template_participant_id: parseExecutionTemplateParticipantId(value.template_participant_id),
    source_agent_id: parseAgentId(value.source_agent_id),
    preset_id: value.preset_id as TAgentExecutionTemplateParticipant['preset_id'],
    provider_id: value.provider_id == null ? null : parseProviderId(value.provider_id),
  };
};

const fromApiExecutionTemplateDetail = (raw: unknown): TAgentExecutionTemplateDetail => {
  const value = executionWireObject(raw);
  return {
    ...fromApiExecutionTemplate(value),
    participants: (value.participants as unknown[]).map(fromApiExecutionTemplateParticipant),
  };
};

export const agentExecution = {
  list: withResponseMap(
    httpGet<unknown[], void>('/api/agent-executions'),
    (raw): TAgentExecution[] => raw.map(fromApiAgentExecution)
  ),
  create: withResponseMap(
    httpPost<unknown, TCreateAgentExecution>('/api/agent-executions'),
    fromApiAgentExecution
  ),
  get: withResponseMap(
    httpGet<unknown, { execution_id: ExecutionId }>((p) => `/api/agent-executions/${p.execution_id}`),
    fromApiAgentExecutionDetail
  ),
  remove: httpDelete<void, { execution_id: ExecutionId; expected_version: number }>(
    (p) => `/api/agent-executions/${p.execution_id}?expected_version=${p.expected_version}`
  ),
  rename: withResponseMap(
    httpPatch<unknown, { execution_id: ExecutionId; updates: TRenameAgentExecution }>(
      (p) => `/api/agent-executions/${p.execution_id}/rename`,
      (p) => p.updates
    ),
    fromApiAgentExecution
  ),
  replan: withResponseMap(
    httpPost<unknown, { execution_id: ExecutionId; updates: TReplanAgentExecution }>(
      (p) => `/api/agent-executions/${p.execution_id}/replan`,
      (p) => p.updates
    ),
    fromApiAgentExecutionDetail
  ),
  adjust: withResponseMap(
    httpPost<unknown, { execution_id: ExecutionId; updates: TAdjustAgentExecution }>(
      (p) => `/api/agent-executions/${p.execution_id}/adjust`,
      (p) => p.updates
    ),
    fromApiAgentExecutionDetail
  ),
  approve: withResponseMap(
    httpPost<unknown, { execution_id: ExecutionId; updates: TVersionedAgentExecutionCommand }>(
      (p) => `/api/agent-executions/${p.execution_id}/approve`,
      (p) => p.updates
    ),
    fromApiAgentExecution
  ),
  pause: withResponseMap(
    httpPost<unknown, { execution_id: ExecutionId; updates: TVersionedAgentExecutionCommand }>(
      (p) => `/api/agent-executions/${p.execution_id}/pause`,
      (p) => p.updates
    ),
    fromApiAgentExecution
  ),
  resume: withResponseMap(
    httpPost<unknown, { execution_id: ExecutionId; updates: TVersionedAgentExecutionCommand }>(
      (p) => `/api/agent-executions/${p.execution_id}/resume`,
      (p) => p.updates
    ),
    fromApiAgentExecution
  ),
  cancel: withResponseMap(
    httpPost<unknown, { execution_id: ExecutionId; updates: TVersionedAgentExecutionCommand }>(
      (p) => `/api/agent-executions/${p.execution_id}/cancel`,
      (p) => p.updates
    ),
    fromApiAgentExecutionDetail
  ),
  addSteps: withResponseMap(
    httpPost<unknown, { execution_id: ExecutionId; updates: TAddExecutionSteps }>(
      (p) => `/api/agent-executions/${p.execution_id}/steps`,
      (p) => p.updates
    ),
    fromApiAgentExecutionDetail
  ),
  updateStep: withResponseMap(
    httpPatch<unknown, { execution_id: ExecutionId; step_id: ExecutionStepId; updates: TUpdateExecutionStep }>(
      (p) => `/api/agent-executions/${p.execution_id}/steps/${p.step_id}`,
      (p) => p.updates
    ),
    fromApiExecutionStep
  ),
  reassign: withResponseMap(
    httpPut<unknown, { execution_id: ExecutionId; step_id: ExecutionStepId; updates: TReassignExecutionStep }>(
      (p) => `/api/agent-executions/${p.execution_id}/steps/${p.step_id}/reassign`,
      (p) => p.updates
    ),
    fromApiExecutionStep
  ),
  steer: httpPost<void, { execution_id: ExecutionId; step_id: ExecutionStepId; updates: TSteerExecutionStep }>(
    (p) => `/api/agent-executions/${p.execution_id}/steps/${p.step_id}/steer`,
    (p) => p.updates
  ),
  retry: withResponseMap(
    httpPost<unknown, { execution_id: ExecutionId; step_id: ExecutionStepId; updates: TRetryExecutionStep }>(
      (p) => `/api/agent-executions/${p.execution_id}/steps/${p.step_id}/retry`,
      (p) => p.updates
    ),
    fromApiAgentExecutionDetail
  ),
  adopt: withResponseMap(
    httpPost<
      unknown,
      {
        execution_id: ExecutionId;
        step_id: ExecutionStepId;
        updates: TAdoptExecutionStepOutput;
      }
    >(
      (p) => `/api/agent-executions/${p.execution_id}/steps/${p.step_id}/adopt`,
      (p) => p.updates
    ),
    fromApiAgentExecutionDetail
  ),
  configure: withResponseMap(
    httpPatch<unknown, { execution_id: ExecutionId; step_id: ExecutionStepId; updates: TConfigureExecutionStep }>(
      (p) => `/api/agent-executions/${p.execution_id}/steps/${p.step_id}/configure`,
      (p) => p.updates
    ),
    fromApiExecutionStep
  ),
  answerDecision: withResponseMap(
    httpPost<
      unknown,
      {
        execution_id: ExecutionId;
        step_id: ExecutionStepId;
        attempt_id: ExecutionAttemptId;
        updates: TAnswerExecutionDecision;
      }
    >(
      (p) => `/api/agent-executions/${p.execution_id}/steps/${p.step_id}/attempts/${p.attempt_id}/answer`,
      (p) => p.updates
    ),
    fromApiAgentExecutionDetail
  ),
  listEvents: withResponseMap(httpGet<unknown[], { execution_id: ExecutionId; query?: TAgentExecutionEventsQuery }>((p) => {
    const params = new URLSearchParams();
    if (p.query?.after_sequence !== undefined) {
      params.set('after_sequence', String(p.query.after_sequence));
    }
    if (p.query?.limit !== undefined) params.set('limit', String(p.query.limit));
    const query = params.toString();
    return `/api/agent-executions/${p.execution_id}/events${query ? `?${query}` : ''}`;
  }), (raw): TAgentExecutionEvent[] => raw.map(fromApiAgentExecutionEvent)),
  getWorkspace: {
    provider: () => {},
    invoke: (async (p: { execution_id: ExecutionId; work_dir: string; path: string; search?: string }) => {
      const rel = absoluteToRelativePath(p.path, p.work_dir);
      const url = `/api/agent-executions/${p.execution_id}/workspace?path=${encodeURIComponent(rel)}${p.search ? `&search=${encodeURIComponent(p.search)}` : ''}`;
      const raw = await httpRequest<Array<{ name: string; type: string }>>('GET', url);
      return fromBackendWorkspaceList(raw, p.work_dir, rel);
    }) as (p: { execution_id: ExecutionId; work_dir: string; path: string; search?: string }) => Promise<IDirOrFile[]>,
  },
  events: {
    changed: wsMappedEmitter<TAgentExecutionChangedEvent>('agentExecution.changed', (raw) => {
      const value = executionWireObject(raw);
      return { ...(value as unknown as TAgentExecutionChangedEvent), execution_id: parseExecutionId(value.execution_id) };
    }),
    leadThinking: wsMappedEmitter<TAgentExecutionLeadThinkingEvent>('agentExecution.leadThinking', (raw) => {
      const value = executionWireObject(raw);
      return { ...(value as unknown as TAgentExecutionLeadThinkingEvent), execution_id: parseExecutionId(value.execution_id) };
    }),
  },
};

// Reusable collaboration authoring input. Templates never become runtime
// state; createExecution copies them once into the canonical execution model.
export const agentExecutionTemplate = {
  list: withResponseMap(
    httpGet<unknown[], void>('/api/agent-execution-templates'),
    (raw): TAgentExecutionTemplate[] => raw.map(fromApiExecutionTemplate)
  ),
  get: withResponseMap(
    httpGet<unknown, { execution_template_id: ExecutionTemplateId }>(
      (p) => `/api/agent-execution-templates/${p.execution_template_id}`
    ),
    fromApiExecutionTemplateDetail
  ),
  create: withResponseMap(
    httpPost<unknown, TCreateAgentExecutionTemplate>('/api/agent-execution-templates'),
    fromApiExecutionTemplateDetail
  ),
  update: withResponseMap(
    httpPut<unknown, { execution_template_id: ExecutionTemplateId; updates: TUpdateAgentExecutionTemplate }>(
      (p) => `/api/agent-execution-templates/${p.execution_template_id}`,
      (p) => p.updates
    ),
    fromApiExecutionTemplateDetail
  ),
  remove: httpDelete<void, { execution_template_id: ExecutionTemplateId; expected_version: number }>(
    (p) => `/api/agent-execution-templates/${p.execution_template_id}?expected_version=${p.expected_version}`
  ),
  createExecution: withResponseMap(
    httpPost<unknown, { execution_template_id: ExecutionTemplateId; request: TCreateExecutionFromTemplate }>(
      (p) => `/api/agent-execution-templates/${p.execution_template_id}/create-execution`,
      (p) => p.request
    ),
    fromApiAgentExecution
  ),
};
// ─────────────────────────── Companion (nomi 桌面伙伴) ───────────────────────────

export interface ICompanionCollectConfig {
  chat_user_messages: boolean;
  requirements: boolean;
  terminal_sessions: boolean;
  tool_calls: boolean;
  companion_dialogues: boolean;
  event_retention_days: number;
  event_max_storage_mb: number;
}

export type ICompanionMemoryKind = 'profile' | 'preference' | 'knowledge' | 'episode' | 'task' | 'affective';

export interface ICompanionMemory {
  memory_id: CompanionMemoryId;
  kind: ICompanionMemoryKind;
  content: string;
  tags: string[];
  importance: number;
  strength: number;
  pinned: boolean;
  source: string;
  status: 'active' | 'archived';
  created_at: number;
  updated_at: number;
  last_reinforced_at: number;
  /**
   * The companion this memory belongs to. Memory is strictly per-companion —
   * there is no shared/install-wide scope any more, and no way to re-home a
   * memory. `null` is only the vestigial state of a legacy row the backend's
   * one-time re-homing migration has not reached yet (it runs at every launch
   * that has at least one companion), so no surface should build behaviour on it
   * beyond "belongs to nobody in particular yet".
   *
   * This is the WHOLE answer to "whose memory is this": the `scope_kind`
   * discriminator that used to travel beside it is gone from the wire and from
   * the database (one nullable owner column, `companion_memories.companion_id`),
   * and the field itself no longer travels under its historical name
   * `scope_companion_id` — wire, column and contract now agree.
   */
  companion_id: CompanionId | null;
  /** FTS highlight snippet (`<b>…</b>` markers) — list results of a full-text query only. */
  snippet?: string | null;
  /** Fused relevance rank — list results of a full-text query only. */
  rank?: number | null;
}

export interface ICompanionMemoryPage {
  items: ICompanionMemory[];
  total: number;
}

/** Sort orders of the memory list (relevance needs a full-text `q`). */
export type ICompanionMemorySort = 'relevance' | 'time' | 'importance';

/** Atomic batch operations over memories (single server-side transaction). */
export type ICompanionMemoryBatchAction = 'archive' | 'restore' | 'delete' | 'reclassify';

/** One suspected-duplicate cluster from the merge assistant's dry run. */
export interface ICompanionMemoryMergeGroup {
  memories: ICompanionMemory[];
}

/** A companion's self-evolved skill (registry row + SKILL.md description). snake_case = Rust JSON 1:1. */
export interface ICompanionSkill {
  companion_skill_id: CompanionSkillId;
  skill_name: string;
  /**
   * The companion this skill belongs to. A self-evolved skill is strictly
   * per-companion — there is no shared tier and no way to hand one to another
   * companion. `null` is only the vestigial state of a legacy row the backend's
   * one-time re-homing migration has not claimed yet.
   *
   * Named after the column it comes from (`companion_skills.companion_id`), like
   * the memory owner above: the field used to travel as `scope_companion_id`, and
   * that spelling is now gone from the wire as well as from the database.
   */
  companion_id: CompanionId | null;
  status: 'draft' | 'active' | 'archived';
  source: string;
  confidence: number;
  provenance_event_ids: CompanionEventId[];
  strength: number;
  version: number;
  skill_pattern_id: SkillPatternId | null;
  usage_count: number;
  last_used_at: number | null;
  created_at: number;
  updated_at: number;
  description: string; // from SKILL.md frontmatter (CompanionSkillView)
}

export interface ICompanionSkillPage {
  items: ICompanionSkill[];
  total: number;
}

export interface ICompanionSkillContent {
  skill: ICompanionSkill;
  content: string;
}

/** WS payload for companion.skill-drafted / companion.skill-learned. */
export interface ICompanionSkillEvent {
  companion_id: CompanionId;
  companion_skill_id: CompanionSkillId;
  skill_name: string;
}

export interface ICompanionLearnResult {
  status: string;
  events_processed: number;
  memories_added: number;
  error?: string | null;
  summary?: string | null;
}

export interface ICompanionStatus {
  /** Owning companion id, or null for the shared-only no-companions fallback. */
  companion_id: CompanionId | null;
  xp: number;
  level: number;
  mood: string;
  memories_active: number;
  memories_archived: number;
  model_configured: boolean;
  collect_any_enabled: boolean;
}

/** "What I learned this week" digest for one companion (skills + memories it gained). */
export interface ICompanionWeeklyDigest {
  since_ms: number;
  skills_learned: number;
  memories_added: number;
  new_skill_names: string[];
}

export interface ICompanionSourceStats {
  source: string;
  today: number;
  total: number;
}

export interface ICompanionEventStorageStatus {
  total_bytes: number;
  max_bytes: number;
  file_count: number;
  oldest_day: string | null;
  newest_day: string | null;
  retention_days: number;
  max_storage_mb: number;
}

/** One archived session-window day-digest (伙伴会话归档回看). */
export interface ICompanionDayDigest {
  session_window_id: CompanionSessionWindowId;
  companion_id: CompanionId;
  conversation_id: ConversationId;
  /** Local start day, `YYYYMMDD`. */
  session_day: string;
  started_at: number;
  last_activity_at: number;
  closed_at: number | null;
  status: string;
  message_count: number;
  boundary_ts: number;
  /** The compressed narrative summary (markdown). */
  digest: string | null;
  /** JSON string: `{topics,decisions,todos,mood}`. */
  highlights: string | null;
  token_estimate: number;
}

/** One day of a companion's readable history (聊天历史 的日期索引), newest first.
 *  Server-side and complete: `day` is the backend's LOCAL calendar day, the same
 *  key `ICompanionDayDigest.session_day` uses, so the digest marker can never
 *  attach to the wrong day near midnight. */
export interface ICompanionHistoryDay {
  /** Local calendar day, `YYYYMMDD`. */
  day: string;
  /** Visible messages persisted that day. */
  message_count: number;
  /** 会话归档 left a diary on that day. */
  has_digest: boolean;
}

/** 伙伴的唯一专属会话 — 一条真实的 `type='nomi'` 会话。每个伙伴生命周期内恒一条。 */
export interface ICompanionThread {
  conversation_id: ConversationId;
  companion_id: CompanionId;
  title: string;
  created_at: number;
  updated_at: number;
}

// ── Multi-companion (spec docs/superpowers/specs/2026-06-11-unified-memory-knowledge-design.md §4.3/§4.7/§4.8) ──

/** Persona of one companion (same shape as the legacy single-companion config persona). */
export interface ICompanionPersona {
  preset: string;
  custom: string;
}

/** Model reference (provider + model id) as stored in companion configs. */
export interface ICompanionModelRef {
  provider_id: ProviderId;
  model: string;
  use_model?: string | null;
}

/** One companion's speech-synthesis选择: catalog model + provider voice id. */
export interface ICompanionTtsSelection {
  provider_id: ProviderId;
  model: string;
  /** Provider voice id (free text); `null` = the provider's own default voice. */
  voice: string | null;
}

/**
 * One companion's voice-activity-detection tuning. The engine runs locally, so
 * there is no Provider reference here — only tuning. `engine` is a string
 * rather than a union because the backend recognises exactly `'silero'` today
 * and falls back to its built-in energy detector for anything else; a union
 * would make a future engine a breaking type change.
 */
export interface ICompanionVadConfig {
  engine: string;
  /** Speech-probability threshold, 0..1. */
  sensitivity: number;
  /** Trailing silence (ms) that closes one utterance, 200..3000. */
  min_silence_ms: number;
}

/** One companion's voice stack. `asr`/`tts` null = use the install-wide default. */
export interface ICompanionVoiceConfig {
  asr: ICompanionModelRef | null;
  tts: ICompanionTtsSelection | null;
  vad: ICompanionVadConfig;
}

/** Desktop-companion window settings of one companion (`character` lives on ICompanionProfile). */
export interface ICompanionWindowConfig {
  companion_enabled: boolean;
  companion_x?: number | null;
  companion_y?: number | null;
  quiet_start: string;
  quiet_end: string;
  /** DIY single-image figure metadata (`character === 'custom'`); absent for roster characters.
   *  `null` in a patch clears it (RFC 7396) — used when switching back to a built-in character. */
  custom_figure?: {
    aspect: number;
    head_box: { x: number; y: number; w: number; h?: number };
    size_tier: 's' | 'm' | 'l';
    /** Per-companion continuous figure-height override (logical px); supersedes `size_tier`
     *  for this companion's desktop window. Absent ⇒ fall back to the tier. `null` in a patch
     *  clears it (RFC 7396) — used by the 总览 size slider's reset. */
    size_px?: number | null;
    /** Stable library figure UUIDv7 backing this companion; absent for legacy per-companion figures. */
    figure_id?: FigureId;
  } | null;
}

/** A reusable figure in the shared custom-figure library (decoupled from companions). */
export interface IFigureMeta {
  figure_id: FigureId;
  name: string;
  aspect: number;
  head_box: { x: number; y: number; w: number; h?: number };
  size_tier: 's' | 'm' | 'l';
  created_at: number;
}

export type IFigureUpdatePatch = {
  figure_id: FigureId;
  name?: string;
  head_box?: { x: number; y: number; w: number; h: number };
  size_tier?: 's' | 'm' | 'l';
};

/** One companion's profile — `companions/{companion_id}/config.json`. */
export interface ICompanionSkillConfig {
  enabled: string[];
  disabled_auto: string[];
}

export interface ICompanionProfile {
  companion_id: CompanionId;
  /** Positive dataset-local display ordinal. */
  seq: number;
  name: string;
  /** Character id (mochi/ink/roux/pixel/bolt/boo); unknown → default. */
  character: string;
  persona: ICompanionPersona;
  model: ICompanionModelRef | null;
  /** 备用对话模型: replayed once when the main model's turn fails. */
  fallback_model: ICompanionModelRef | null;
  /** 视觉大模型; null = use the main chat model when it can see images. */
  vision_model: ICompanionModelRef | null;
  /** ASR / TTS / VAD for this companion. */
  voice: ICompanionVoiceConfig;
  /** This companion's own 定时学习 loop (install-wide until 2026-08). */
  learn: ICompanionLearnConfig;
  /** This companion's own 技能进化 loop (install-wide until 2026-08). */
  evolve: ICompanionEvolveConfig;
  skills: ICompanionSkillConfig;
  appearance: ICompanionWindowConfig;
  /** Frozen execution configuration last applied to this companion. */
  applied_preset?: ResolvedPresetSnapshot;
  /**
   * User-chosen sidebar position. Absent = never reordered; such companions sort
   * after every explicitly ordered one, by creation time. Distinct from `seq`,
   * which is a registry-owned never-reused display ordinal.
   */
  order_index?: number | null;
  created_at: number;
}

/** One companion's periodic-learning settings (定时学习). */
export interface ICompanionLearnConfig {
  enabled: boolean;
  /** 5..=1440. */
  interval_minutes: number;
  model: ICompanionModelRef | null;
}

/**
 * One companion's skill-evolution settings (技能进化).
 *
 * Every field the backend serializes is listed, including the four tuning knobs
 * the UI deliberately never surfaces (`min_pattern_count`, `auto_threshold`,
 * `skill_half_life_days`, `skill_archive_threshold`) — the wire shape is the
 * truth, and omitting a field here just makes the type lie about what arrives.
 * `auto_activate` IS the 保守/激进 preference the 进化 tab renders.
 */
export interface ICompanionEvolveConfig {
  enabled: boolean;
  interval_minutes: number;
  model: ICompanionModelRef | null;
  min_pattern_count: number;
  min_distinct_sessions: number;
  auto_activate: boolean;
  auto_threshold: number;
  skill_half_life_days: number;
  skill_archive_threshold: number;
}

/** Shared session-window archiving settings (伙伴会话窗口归档). Default OFF (opt-in). */
export interface ICompanionArchiveConfig {
  enabled: boolean;
  idle_minutes: number;
  min_chars: number;
  inject_recent_days: number;
}

/**
 * Machine-level (cross-companion) config — `shared/config.json`, served by
 * /api/companion/config.
 *
 * `learn` / `evolve` used to live here, which is what made the 进化 tab
 * install-wide; they are per companion on {@link ICompanionProfile} since
 * 2026-08. What remains is genuinely machine-level: which events this device
 * records, the session archiver, and the default-companion pointer.
 */
export interface ICompanionSharedConfig {
  collect: ICompanionCollectConfig;
  /** Session-window archiving (伙伴会话归档). */
  archive: ICompanionArchiveConfig;
  /** 智能协作：开启后本地伙伴可把复杂任务拆给多个协作者并行推进。 */
  smart_collaboration: boolean;
  /** Null when no companion exists yet (zero-companion state is allowed). */
  default_companion_id: CompanionId | null;
  /**
   * Opt-in (default null = off): when set to a directory path, companion `save`
   * memories are ALSO mirrored into the nomi agent's file-memory there, so the
   * agent recalls companion-learned facts.
   */
  bridge_to_memory_dir: string | null;
}

export type ICompanionWithStatus = ICompanionProfile & {
  status: ICompanionStatus;
};

/// RFC 7396 merge patch over ICompanionProfile — nested partial objects merge.
export type ICompanionProfilePatch = {
  name?: string;
  character?: string;
  persona?: Partial<ICompanionPersona>;
  model?: ICompanionModelRef | null;
  fallback_model?: ICompanionModelRef | null;
  vision_model?: ICompanionModelRef | null;
  voice?: {
    asr?: ICompanionModelRef | null;
    tts?: ICompanionTtsSelection | null;
    vad?: Partial<ICompanionVadConfig>;
  };
  learn?: Partial<ICompanionLearnConfig>;
  evolve?: Partial<ICompanionEvolveConfig>;
  skills?: Partial<ICompanionSkillConfig>;
  appearance?: Partial<ICompanionWindowConfig>;
  order_index?: number | null;
};

/// RFC 7396 merge patch over ICompanionSharedConfig — nested partial objects merge.
export type ICompanionSharedConfigPatch = {
  collect?: Partial<ICompanionCollectConfig>;
  archive?: Partial<ICompanionArchiveConfig>;
  smart_collaboration?: boolean;
  bridge_to_memory_dir?: string | null;
};

/** Export endpoint result — backend echoes the resolved destination path
 *  (plus backend-reported stats; contract still settling). */
export interface ICompanionExportResult {
  dest_path: string;
  [extra: string]: unknown;
}

// WS event payloads — per-companion events carry `companion_id` (spec §4.3).

/** `companion.config-updated` — `scope` distinguishes a shared-config change
 *  (`'shared'`) from a per-companion profile change (`scope === companion_id`, payload =
 *  the full companion profile); `companion_id` is set for per-companion scope. The payload
 *  remainder is scope-dependent, hence the open index signature. */
export interface ICompanionConfigUpdatedEvent {
  scope?: 'shared' | CompanionId;
  companion_id?: CompanionId;
  /** Scope-dependent payload remainder (shared config or full companion profile). */
  [extra: string]: unknown;
}

/** `companion.created` */
export interface ICompanionCreatedEvent {
  companion_id: CompanionId;
  profile: ICompanionProfile;
}

/** `companion.deleted` */
export interface ICompanionDeletedEvent {
  companion_id: CompanionId;
}

const asWireObject = (value: unknown, label: string): Record<string, unknown> => {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError(`${label} must be a JSON object`);
  }
  return value as Record<string, unknown>;
};

const nullableCompanionId = (value: unknown): CompanionId | null =>
  value == null ? null : parseCompanionId(value);

const fromApiCompanionMemory = (raw: unknown): ICompanionMemory => {
  const value = asWireObject(raw, 'companion memory');
  for (const retiredField of ['scope_kind', 'scope_companion_id']) {
    // `scope_kind` was the shared/private discriminator and `scope_companion_id`
    // the owner's historical name; both are gone from the memory wire, which now
    // spells the owner exactly like its column (`companion_id`). Rejecting them
    // is what stops a downgraded backend from quietly serving memories whose
    // owner this adapter would then read as `undefined` — the same guard
    // `fromApiCompanionSkill` has always had.
    if (Object.prototype.hasOwnProperty.call(value, retiredField)) {
      throw new TypeError(`companion memory must not contain retired field "${retiredField}"`);
    }
  }
  return {
    ...(value as unknown as ICompanionMemory),
    memory_id: parseCompanionMemoryId(value.memory_id),
    companion_id: nullableCompanionId(value.companion_id),
  };
};

const fromApiCompanionSkill = (raw: unknown): ICompanionSkill => {
  const value = asWireObject(raw, 'companion skill');
  for (const retiredField of ['provenance', 'superseded_by', 'scope_kind', 'scope_companion_id']) {
    // `scope_kind` was the shared/private discriminator; 共享技能 is gone and the
    // owner alone answers "whose skill is this", so the backend must not send it.
    // `scope_companion_id` was that owner's own historical name, which outlived the
    // column rename as a bare `#[serde(rename)]`. Rejecting it rather than ignoring
    // it is what stops a mismatched backend from serving skills whose owner every
    // caller then reads as `undefined` — the guard the memory adapter mirrors.
    if (Object.prototype.hasOwnProperty.call(value, retiredField)) {
      throw new TypeError(`companion skill must not contain retired field "${retiredField}"`);
    }
  }
  if (!Array.isArray(value.provenance_event_ids)) {
    throw new TypeError('companion skill provenance_event_ids must be an array');
  }
  return {
    ...(value as unknown as ICompanionSkill),
    companion_skill_id: parseCompanionSkillId(value.companion_skill_id),
    companion_id: nullableCompanionId(value.companion_id),
    provenance_event_ids: value.provenance_event_ids.map(parseCompanionEventId),
    skill_pattern_id:
      value.skill_pattern_id == null ? null : parseSkillPatternId(value.skill_pattern_id),
  };
};

const fromApiCompanionLearnResult = (raw: unknown): ICompanionLearnResult => {
  const value = asWireObject(raw, 'companion learn result');
  for (const retiredField of ['learn_run_id', 'started_at', 'finished_at']) {
    if (Object.prototype.hasOwnProperty.call(value, retiredField)) {
      throw new TypeError(`companion learn result must not contain retired history field "${retiredField}"`);
    }
  }
  if (typeof value.status !== 'string') {
    throw new TypeError('companion learn result status must be a string');
  }
  for (const countField of ['events_processed', 'memories_added'] as const) {
    if (typeof value[countField] !== 'number' || !Number.isSafeInteger(value[countField])) {
      throw new TypeError(`companion learn result ${countField} must be a safe integer`);
    }
  }
  for (const textField of ['error', 'summary'] as const) {
    if (value[textField] != null && typeof value[textField] !== 'string') {
      throw new TypeError(`companion learn result ${textField} must be a string or null`);
    }
  }
  return {
    status: value.status,
    events_processed: value.events_processed as number,
    memories_added: value.memories_added as number,
    error: value.error as string | null | undefined,
    summary: value.summary as string | null | undefined,
  };
};

const fromApiCompanionStatus = (raw: unknown): ICompanionStatus => {
  const value = asWireObject(raw, 'companion status');
  return {
    ...(value as unknown as ICompanionStatus),
    companion_id: nullableCompanionId(value.companion_id),
  };
};

const fromApiCompanionWindowConfig = (raw: unknown): ICompanionWindowConfig => {
  const value = asWireObject(raw, 'companion appearance');
  if (value.custom_figure == null) {
    return value as unknown as ICompanionWindowConfig;
  }
  const customFigure = asWireObject(value.custom_figure, 'companion custom figure');
  return {
    ...(value as unknown as ICompanionWindowConfig),
    custom_figure: {
      ...(customFigure as unknown as NonNullable<ICompanionWindowConfig['custom_figure']>),
      ...(customFigure.figure_id == null ? {} : { figure_id: parseFigureId(customFigure.figure_id) }),
    },
  };
};

const fromApiCompanionProfile = (raw: unknown): ICompanionProfile => {
  const value = asWireObject(raw, 'companion profile');
  if ('id' in value) {
    throw new TypeError('companion profile wire payload must use companion_id, not id');
  }
  return {
    ...(value as unknown as ICompanionProfile),
    companion_id: parseCompanionId(value.companion_id),
    appearance: fromApiCompanionWindowConfig(value.appearance),
  };
};

const fromApiCompanionWithStatus = (raw: unknown): ICompanionWithStatus => {
  const value = asWireObject(raw, 'companion profile with status');
  return {
    ...fromApiCompanionProfile(value),
    status: fromApiCompanionStatus(value.status),
  };
};

const fromApiCompanionDayDigest = (raw: unknown): ICompanionDayDigest => {
  const value = asWireObject(raw, 'companion day digest');
  return {
    ...(value as unknown as ICompanionDayDigest),
    session_window_id: parseCompanionSessionWindowId(value.session_window_id),
    companion_id: parseCompanionId(value.companion_id),
    conversation_id: parseConversationId(value.conversation_id),
  };
};

const fromApiFigure = (raw: unknown): IFigureMeta => {
  const value = asWireObject(raw, 'companion figure');
  if ('id' in value) {
    throw new TypeError('figure wire payload must use figure_id, not id');
  }
  return { ...(value as unknown as IFigureMeta), figure_id: parseFigureId(value.figure_id) };
};

const fromApiCompanionThread = (raw: unknown): ICompanionThread => {
  const value = asWireObject(raw, 'companion thread');
  return {
    ...(value as unknown as ICompanionThread),
    companion_id: parseCompanionId(value.companion_id),
    conversation_id: parseConversationId(value.conversation_id),
  };
};

const fromApiCompanionSharedConfig = (raw: unknown): ICompanionSharedConfig => {
  const value = asWireObject(raw, 'companion shared config');
  return {
    ...(value as unknown as ICompanionSharedConfig),
    default_companion_id: nullableCompanionId(value.default_companion_id),
  };
};

export const companion = {
  /**
   * `companion_id` narrows the list to ONE companion's memories (plus any
   * legacy row not yet re-homed). Omitting it returns every companion's memories
   * and is only for an owner-level administrative view.
   */
  listMemories: withResponseMap(
    httpGet<
      { items: unknown[]; total: number },
      {
      kind?: string;
      q?: string;
      status?: string;
      companion_id?: CompanionId;
      sort?: ICompanionMemorySort;
      limit?: number;
      offset?: number;
      }
    >((p) => {
      const params = new URLSearchParams();
      if (p?.kind) params.set('kind', p.kind);
      if (p?.q) params.set('q', p.q);
      if (p?.status) params.set('status', p.status);
      if (p?.companion_id) params.set('companion_id', p.companion_id);
      if (p?.sort) params.set('sort', p.sort);
      if (p?.limit) params.set('limit', String(p.limit));
      if (p?.offset) params.set('offset', String(p.offset));
      const qs = params.toString();
      return `/api/companion/memories${qs ? `?${qs}` : ''}`;
    }),
    (raw): ICompanionMemoryPage => ({ ...raw, items: raw.items.map(fromApiCompanionMemory) })
  ),
  /** `companion_id` is the OWNER of the new memory (omitted = server-resolved). */
  addMemory: withResponseMap(
    httpPost<unknown, { kind: string; content: string; tags?: string[]; companion_id?: CompanionId }>(
      '/api/companion/memories'
    ),
    fromApiCompanionMemory
  ),
  /**
   * Content / pin / lifecycle only: a memory's owner is fixed at write time.
   *
   * `companion_id` is the companion DOING the edit, not a new owner — the
   * store rejects a row owned by anyone else with a 404. Required on every memory
   * mutation below for the same reason: the invariant is enforced server-side, so
   * the caller has to say who it is.
   */
  updateMemory: httpPut<
    void,
    {
      memory_id: CompanionMemoryId;
      companion_id: CompanionId;
      content?: string;
      pinned?: boolean;
      status?: string;
    }
  >(
    (p) => `/api/companion/memories/${p.memory_id}`,
    (p) => ({
      content: p.content,
      pinned: p.pinned,
      status: p.status,
      companion_id: p.companion_id,
    })
  ),
  deleteMemory: httpDelete<void, { memory_id: CompanionMemoryId; companion_id: CompanionId }>(
    (p) => `/api/companion/memories/${p.memory_id}?companion_id=${encodeURIComponent(p.companion_id)}`
  ),
  /** Atomic batch memory op (single transaction — any bad or foreign id rolls the whole batch back). */
  batchMemories: httpPost<
    void,
    {
      ids: CompanionMemoryId[];
      action: ICompanionMemoryBatchAction;
      kind?: ICompanionMemoryKind;
      companion_id: CompanionId;
    }
  >('/api/companion/memories/batch'),
  /**
   * Merge-assistant dry run: suspected-duplicate groups over the active layer of
   * ONE companion. Scoped server-side — the response carries memory content, so
   * this surface never receives another companion's text to filter out.
   */
  memoryMergeSuggestions: withResponseMap(
    httpPost<unknown[], { companion_id: CompanionId }>('/api/companion/memories/merge-suggestions'),
    (raw): ICompanionMemoryMergeGroup[] =>
      raw.map((entry) => {
        const value = asWireObject(entry, 'companion memory merge group');
        if (!Array.isArray(value.memories)) {
          throw new TypeError('companion memory merge group memories must be an array');
        }
        return { memories: value.memories.map(fromApiCompanionMemory) };
      })
  ),
  /** Merge-assistant confirm: insert the merged memory, archive the source group. */
  mergeMemories: withResponseMap(
    httpPost<
      unknown,
      {
        group: CompanionMemoryId[];
        merged_content: string;
        kind: ICompanionMemoryKind;
        companion_id: CompanionId;
      }
    >('/api/companion/memories/merge'),
    fromApiCompanionMemory
  ),
  // ── Self-evolved skills (P2: see + edit). Addressed by companion_id + companion_skill_id. ──
  /** One companion's own skills — the companion in the path IS the whole scope. */
  listSkills: withResponseMap(
    httpGet<
      { items: unknown[]; total: number },
      {
      companion_id: CompanionId;
      status?: string;
      limit?: number;
      offset?: number;
      }
    >((p) => {
      const params = new URLSearchParams();
      if (p.status) params.set('status', p.status);
      if (p.limit) params.set('limit', String(p.limit));
      if (p.offset) params.set('offset', String(p.offset));
      const qs = params.toString();
      return `/api/companion/companions/${p.companion_id}/skills${qs ? `?${qs}` : ''}`;
    }),
    (raw): ICompanionSkillPage => ({ ...raw, items: raw.items.map(fromApiCompanionSkill) })
  ),
  getSkillContent: withResponseMap(
    httpGet<
      { skill: unknown; content: string },
      { companion_id: CompanionId; companion_skill_id: CompanionSkillId }
    >(
      (p) =>
        `/api/companion/companions/${p.companion_id}/skills/${p.companion_skill_id}`,
      { silentStatuses: [404] }
    ),
    (raw): ICompanionSkillContent => ({ ...raw, skill: fromApiCompanionSkill(raw.skill) })
  ),
  writeSkillContent: httpPut<
    void,
    { companion_id: CompanionId; companion_skill_id: CompanionSkillId; content: string }
  >(
    (p) =>
      `/api/companion/companions/${p.companion_id}/skills/${p.companion_skill_id}`,
    (p) => ({ content: p.content })
  ),
  decideSkill: withResponseMap(
    httpPost<
      unknown,
      {
        companion_id: CompanionId;
        companion_skill_id: CompanionSkillId;
        accept: boolean;
        reason?: string;
      }
    >(
      (p) =>
        `/api/companion/companions/${p.companion_id}/skills/${p.companion_skill_id}/decide`,
      (p) => ({ accept: p.accept, reason: p.reason })
    ),
    fromApiCompanionSkill
  ),
  weeklyDigest: httpGet<ICompanionWeeklyDigest, { companion_id: CompanionId; days?: number }>(
    (p) => `/api/companion/companions/${p.companion_id}/weekly-digest${p.days ? `?days=${p.days}` : ''}`
  ),
  /** Archived session-window day-digests (伙伴会话归档回看时间线 / 去年今日). */
  listDayDigests: withResponseMap(
    httpGet<
      unknown[],
      {
      companion_id: CompanionId;
      since?: string;
      until?: string;
      on_day?: string;
      limit?: number;
      }
    >((p) => {
      const q = new URLSearchParams();
      if (p.since) q.set('since', p.since);
      if (p.until) q.set('until', p.until);
      if (p.on_day) q.set('on_day', p.on_day);
      if (p.limit) q.set('limit', String(p.limit));
      const qs = q.toString();
      return `/api/companion/companions/${p.companion_id}/digests${qs ? `?${qs}` : ''}`;
    }),
    (raw): ICompanionDayDigest[] => raw.map(fromApiCompanionDayDigest)
  ),
  /** The companion's complete history day index (聊天历史 左侧日期栏). Read-only:
   *  never mints a session, and a companion that has never chatted returns []. */
  listHistoryDays: withResponseMap(
    httpGet<unknown[], { companion_id: CompanionId }>(
      (p) => `/api/companion/companions/${p.companion_id}/history/days`
    ),
    (raw): ICompanionHistoryDay[] =>
      raw.map((entry) => asWireObject(entry, 'companion history day') as unknown as ICompanionHistoryDay)
  ),
  /** Learn-by-demonstration: draft a skill from a work session's tool sequence. Returns the name. */
  draftFromSession: httpPost<string | null, { companion_id: CompanionId; conversation_id: ConversationId }>(
    (p) => `/api/companion/companions/${p.companion_id}/skills/from-session`,
    (p) => ({ conversation_id: p.conversation_id })
  ),
  /** Run ONE companion's 定时学习 pass now (the run lock is per companion). */
  runLearn: withResponseMap(
    httpPost<unknown, { companion_id: CompanionId }>(
      (p) => `/api/companion/companions/${p.companion_id}/learn/run`,
      () => ({})
    ),
    fromApiCompanionLearnResult
  ),
  eventStats: httpGet<ICompanionSourceStats[], void>('/api/companion/events/stats'),
  eventStorage: httpGet<ICompanionEventStorageStatus, void>('/api/companion/events/storage'),
  /** First-launch consent: apply self-evolution default-ON once (server KV-gated). */
  applyConsent: httpPost<ICompanionSharedConfig, void>('/api/companion/consent'),
  /** Master kill switch: stop all collection + learning + evolution. */
  disableAll: httpPost<ICompanionSharedConfig, void>('/api/companion/disable-all'),
  // ── Multi-companion CRUD (spec §4.3) ──
  listCompanions: withResponseMap(
    httpGet<unknown[], void>('/api/companion/companions'),
    (raw): ICompanionWithStatus[] => raw.map(fromApiCompanionWithStatus)
  ),
  createCompanion: withResponseMap(
    httpPost<unknown, { name: string; character: string }>('/api/companion/companions'),
    fromApiCompanionProfile
  ),
  getCompanion: withResponseMap(
    httpGet<unknown, { companion_id: CompanionId }>((p) => `/api/companion/companions/${p.companion_id}`),
    fromApiCompanionWithStatus
  ),
  /** RFC 7396 merge patch over one companion's profile (name/character/persona/model/appearance). */
  patchCompanion: withResponseMap(
    httpPatch<unknown, { companion_id: CompanionId; patch: ICompanionProfilePatch }>(
      (p) => `/api/companion/companions/${p.companion_id}`,
      (p) => p.patch
    ),
    fromApiCompanionProfile
  ),
  applyPreset: withResponseMap(
    httpPost<
      unknown,
      { companion_id: CompanionId; preset_id: PresetReference; locale?: string; overrides?: import('../types/agent/presetTypes').PresetOverrides }
    >(
      (p) => `/api/companion/companions/${p.companion_id}/apply-preset`,
      (p) => ({
        preset_id: p.preset_id,
        locale: p.locale,
        overrides: p.overrides ?? {},
      })
    ),
    fromApiCompanionProfile
  ),
  deleteCompanion: httpDelete<void, { companion_id: CompanionId }>((p) => `/api/companion/companions/${p.companion_id}`),
  getCompanionStatus: withResponseMap(
    httpGet<unknown, { companion_id: CompanionId }>((p) => `/api/companion/companions/${p.companion_id}/status`),
    fromApiCompanionStatus
  ),
  /** Ingest a DIY figure image previously landed in the temp upload root via `/api/fs/upload` (two-phase upload). */
  uploadFigure: httpPost<void, { companion_id: CompanionId; source_path: string }>(
    (p) => `/api/companion/companions/${p.companion_id}/figure`,
    (p) => ({ source_path: p.source_path })
  ),
  // ── Custom-figure library (reusable, decoupled from companions) ──
  listFigures: withResponseMap(
    httpGet<unknown[], void>('/api/companion/figures'),
    (raw): IFigureMeta[] => raw.map(fromApiFigure)
  ),
  /** Create a reusable library figure from a temp upload (two-phase upload). */
  createFigure: withResponseMap(
    httpPost<
      unknown,
      { source_path: string; name: string; aspect: number; head_box: { x: number; y: number; w: number; h: number }; size_tier: 's' | 'm' | 'l' }
    >('/api/companion/figures'),
    fromApiFigure
  ),
  updateFigure: withResponseMap(
    httpPatch<unknown, IFigureUpdatePatch>(
      (p) => `/api/companion/figures/${p.figure_id}`,
      (p) => ({ name: p.name, head_box: p.head_box, size_tier: p.size_tier })
    ),
    fromApiFigure
  ),
  renameFigure: withResponseMap(
    httpPatch<unknown, { figure_id: FigureId; name: string }>(
      (p) => `/api/companion/figures/${p.figure_id}`,
      (p) => ({ name: p.name })
    ),
    fromApiFigure
  ),
  deleteFigure: httpDelete<void, { figure_id: FigureId }>((p) => `/api/companion/figures/${p.figure_id}`),
  // ── 伙伴单会话（companion single session）──
  // 每个伙伴生命周期内恒一条专属会话；多线程列表/新建/重命名/单删/设活已废除。
  /** Return this companion's canonical Conversation id, or null. */
  getCompanionSession: withResponseMap(
    httpGet<{ conversation_id: string | null }, { companion_id: CompanionId }>(
      (p) => `/api/companion/companions/${p.companion_id}/companion/active`
    ),
    (raw): { conversation_id: ConversationId | null } => ({
      conversation_id: raw.conversation_id == null ? null : parseConversationId(raw.conversation_id),
    })
  ),
  /** Idempotently ensure the companion's unique canonical Conversation. */
  ensureCompanionSession: withResponseMap(
    httpPost<unknown, { companion_id: CompanionId }>(
      (p) => `/api/companion/companions/${p.companion_id}/companion/threads`,
      () => ({})
    ),
    fromApiCompanionThread
  ),
  // ── Shared (cross-companion) config — same /api/companion/config route, multi-companion shape ──
  getSharedConfig: withResponseMap(
    httpGet<unknown, void>('/api/companion/config'),
    fromApiCompanionSharedConfig
  ),
  patchSharedConfig: withResponseMap(
    httpPatch<unknown, ICompanionSharedConfigPatch>('/api/companion/config'),
    fromApiCompanionSharedConfig
  ),
  // ── Import / export (spec §4.8) ──
  exportMemory: httpPost<ICompanionExportResult, { dest_path: string; include_events: boolean }>('/api/companion/export/memory'),
  /** Export one companion. Its settings are always included; `include_memories`
   *  defaults to true and `include_skills` to false, and a custom figure travels
   *  whenever the companion wears one. */
  exportCompanion: httpPost<
    ICompanionExportResult,
    {
      companion_id: CompanionId;
      dest_path: string;
      knowledge_names?: string[];
      include_memories?: boolean;
      include_skills?: boolean;
    }
  >(
    (p) => `/api/companion/export/companions/${p.companion_id}`,
    (p) => ({
      dest_path: p.dest_path,
      knowledge_names: p.knowledge_names ?? [],
      include_memories: p.include_memories ?? true,
      include_skills: p.include_skills ?? false,
    })
  ),
  /** Import a memory/companion bundle; the backend dispatches on manifest.kind. */
  importCompanionBundle: httpPost<Record<string, unknown>, { src_path: string }>('/api/companion/import'),
  onLearnStarted: wsMappedEmitter<{ companion_id?: CompanionId }>('companion.learn-started', (raw) => {
    const value = asWireObject(raw, 'companion learn-started event');
    return value.companion_id == null ? {} : { companion_id: parseCompanionId(value.companion_id) };
  }),
  onLearnFinished: wsMappedEmitter<ICompanionLearnResult & { companion_id?: CompanionId }>(
    'companion.learn-finished',
    (raw) => {
      const value = asWireObject(raw, 'companion learn-finished event');
      return {
        ...fromApiCompanionLearnResult(value),
        ...(value.companion_id == null ? {} : { companion_id: parseCompanionId(value.companion_id) }),
      };
    }
  ),
  onMoodChanged: wsMappedEmitter<{ mood: string; companion_id?: CompanionId }>('companion.mood-changed', (raw) => {
    const value = asWireObject(raw, 'companion mood-changed event');
    if (typeof value.mood !== 'string') throw new TypeError('companion mood must be a string');
    return {
      mood: value.mood,
      ...(value.companion_id == null ? {} : { companion_id: parseCompanionId(value.companion_id) }),
    };
  }),
  onConfigUpdated: wsMappedEmitter<ICompanionConfigUpdatedEvent>('companion.config-updated', (raw) => {
    const value = asWireObject(raw, 'companion config-updated event');
    const scope = value.scope === 'shared' || value.scope == null ? value.scope : parseCompanionId(value.scope);
    return {
      ...value,
      ...(scope == null ? {} : { scope }),
      ...(value.companion_id == null ? {} : { companion_id: parseCompanionId(value.companion_id) }),
    };
  }),
  onMemoryCreated: wsMappedEmitter<ICompanionMemory>('companion.memory-created', fromApiCompanionMemory),
  onMemoryUpdated: wsMappedEmitter<ICompanionMemory>('companion.memory-updated', fromApiCompanionMemory),
  onMemoryDeleted: wsMappedEmitter<{ memory_id: CompanionMemoryId }>('companion.memory-deleted', (raw) => {
    const value = asWireObject(raw, 'companion memory-deleted event');
    return { memory_id: parseCompanionMemoryId(value.memory_id) };
  }),
  onSkillDrafted: wsMappedEmitter<ICompanionSkillEvent>('companion.skill-drafted', (raw) => {
    const value = asWireObject(raw, 'companion skill-drafted event');
    return {
      companion_id: parseCompanionId(value.companion_id),
      companion_skill_id: parseCompanionSkillId(value.companion_skill_id),
      skill_name: String(value.skill_name),
    };
  }),
  onSkillLearned: wsMappedEmitter<ICompanionSkillEvent>('companion.skill-learned', (raw) => {
    const value = asWireObject(raw, 'companion skill-learned event');
    return {
      companion_id: parseCompanionId(value.companion_id),
      companion_skill_id: parseCompanionSkillId(value.companion_skill_id),
      skill_name: String(value.skill_name),
    };
  }),
  onSkillArchived: wsMappedEmitter<ICompanionSkillEvent>('companion.skill-archived', (raw) => {
    const value = asWireObject(raw, 'companion skill-archived event');
    return {
      companion_id: parseCompanionId(value.companion_id),
      companion_skill_id: parseCompanionSkillId(value.companion_skill_id),
      skill_name: String(value.skill_name),
    };
  }),
  onCompanionCreated: wsMappedEmitter<ICompanionCreatedEvent>('companion.created', (raw) => {
    const value = asWireObject(raw, 'companion created event');
    return {
      companion_id: parseCompanionId(value.companion_id),
      profile: fromApiCompanionProfile(value.profile),
    };
  }),
  onCompanionDeleted: wsMappedEmitter<ICompanionDeletedEvent>('companion.deleted', (raw) => {
    const value = asWireObject(raw, 'companion deleted event');
    return { companion_id: parseCompanionId(value.companion_id) };
  }),
};

/** Phase 2b「登录我的浏览器」status returned by open/close/status. */
export interface IBrowserLoginStatus {
  /** Whether a visible login browser is currently open. */
  active: boolean;
  /** Outcome code: 'opened' | 'already_open' | 'queued' | 'closed' | 'not_open' | 'launch_failed:<err>'. */
  message?: string;
  /** Whether a fresh Primary-identity capture was committed to the encrypted vault
   *  DURING this login session (the Hub advances its canonical identity generation
   *  only after a successful capture + vault persist). NOT a close()-triggered
   *  backup: a manual login that triggered no capture reports `false` even though
   *  the persistent on-disk profile still retains the login for silent reuse. */
  saved: boolean;
  /** Lane backing the login session; present while a session exists. */
  lane_id?: string;
  // NOTE: responses also carry `source` — the EFFECTIVE host-policy Chrome
  // source ('managed' | 'system') the login browser actually uses.
}

/** 「登录我的浏览器」— open a visible browser bound to the shared profile so the user logs
 *  into their sites once; silent agent sessions then reuse the login. The request-body
 *  `source` is IGNORED by the backend (kept only for wire compatibility): the trusted
 *  Chrome source is host policy frozen at process start, and the response's `source`
 *  field reports the effective value (a live agent.browserUse.source toggle only takes
 *  effect after an app restart). */
export const browserLogin = {
  /** Open the visible login window (idempotent while already open; a 'queued'
   *  outcome is foregrounded automatically once the Lane starts running). */
  open: httpPost<IBrowserLoginStatus, { source: 'managed' | 'system' }>('/api/browser/login/open'),
  /** Close it, returns final status; `saved` reports whether a vault capture
   *  actually happened during the session (see IBrowserLoginStatus.saved). */
  close: httpPost<IBrowserLoginStatus, void>('/api/browser/login/close'),
  /** Poll whether a login window is currently open. A pure read: it never
   *  renews or revokes the underlying session. */
  status: httpGet<IBrowserLoginStatus, void>('/api/browser/login/status'),
};

// ==================== Knowledge Base Platform (knowledge) ====================

/** One URL entry of a knowledge-base URL source. */
export interface IKnowledgeSourceEntry {
  sourceItemId?: KnowledgeSourceItemId;
  url: string;
  title?: string;
  /**
   * P3-K3: fetch this URL through the rendering backend (a real headless
   * browser) instead of a plain HTTP GET — for JS-heavy SPAs whose content a
   * static fetch cannot see. Omitted/false ⇒ HTTP (backward compatible). When
   * no browser backend is wired the fetch gracefully falls back to HTTP.
   */
  rendered?: boolean;
  snapshotEntryId?: KnowledgeEntryId;
  syncStatus?: IKnowledgeEntrySource['sync_status'];
  lastSuccessAt?: number;
  lastError?: string;
}

/** 'snapshot' = fetched into managed Markdown entries; 'live' = surfaced to agents as realtime sources. */
export type KnowledgeSourceMode = 'live' | 'snapshot';

/** URL source config of a base (wire shape: camelCase, `lastFetchedAt` epoch-ms). */
export interface IKnowledgeSource {
  sourceId?: KnowledgeSourceId;
  /** Source kind discriminator; "url" is the only kind today. */
  kind: string;
  mode: KnowledgeSourceMode;
  revision?: number;
  defaultParentEntryId?: KnowledgeEntryId;
  entries: IKnowledgeSourceEntry[];
  /** Last successful snapshot fetch (epoch ms); absent until the first fetch. */
  lastFetchedAt?: number;
}

/** Per-batch outcome of a URL-source fetch (create with snapshot source / refresh-source). */
export interface IKnowledgeSourceFetchSummary {
  fetched: number;
  failed: number;
  /** One "{url}: {error}" line per failed entry. */
  errors: string[];
  /** `extra.source.last_fetched_at` after the run; absent when nothing was ever fetched. */
  last_fetched_at?: number;
}

/**
 * Server-authoritative, action-specific policy for one knowledge entry.
 * `origin` explains provenance; callers must use these capabilities for UI and
 * must not infer permissions from a path or source kind.
 */
export interface IKnowledgeEntryCapabilities {
  read_content: boolean;
  edit_content: boolean;
  rename: boolean;
  relocate: boolean;
  accept_children: boolean;
  delete_entry: boolean;
  remove_source: boolean;
  refresh_source: boolean;
  detach_source: boolean;
  copy_as_editable: boolean;
  export_entry: boolean;
  edit_metadata: boolean;
  /** Human-readable explanation when body editing is restricted. */
  read_only_reason?: string;
}

export interface IKnowledgeEntrySource {
  source_id: KnowledgeSourceId;
  source_item_id: KnowledgeSourceItemId;
  source_url: string;
  relationship: 'managed' | 'detached' | 'copy';
  sync_status:
    | 'pending'
    | 'syncing'
    | 'synced'
    | 'failed'
    | 'conflicted'
    | 'missing'
    | 'paused';
  final_url?: string;
  last_success_at?: number;
  last_error?: string;
}

/** Append-only inputs accepted by the unified knowledge-content endpoint. */
export type IKnowledgeAddContentInput =
  | { type: 'document'; path: string; content: string }
  | { type: 'local_folder'; source_path: string; destination_parent_path?: string }
  | {
      type: 'web';
      entries: IKnowledgeSourceEntry[];
      destination_parent_path?: string;
      destination_parent_id?: KnowledgeEntryId;
    };

/**
 * Bridge request shape. Fields stay optional here because the bridge's mapped
 * invoke type does not preserve discriminated unions; the exported input union
 * above remains the canonical per-variant contract and the Rust route enforces
 * the selected variant strictly.
 */
export interface IKnowledgeAddContentRequest {
  knowledge_base_id: KnowledgeBaseId;
  type: IKnowledgeAddContentInput['type'];
  path?: string;
  content?: string;
  source_path?: string;
  destination_parent_path?: string;
  destination_parent_id?: KnowledgeEntryId;
  entries?: IKnowledgeSourceEntry[];
}

/** Per-method outcome of adding content to an existing knowledge base. */
export type IKnowledgeAddContentResult =
  | { type: 'document'; path: string }
  | {
      type: 'local_folder';
      target_directory: string;
      imported: number;
      skipped: number;
      total_size: number;
      first_file?: string;
    }
  | ({
      type: 'web';
      added: number;
      duplicates: number;
      first_file?: string;
    } & IKnowledgeSourceFetchSummary);

/** Result of POST /api/knowledge/bases/{id}/autogen (AI overview generation). */
export interface IKnowledgeAutogenOutcome {
  /** The (possibly clamped) description after the run. */
  description: string;
  description_updated: boolean;
  /** Whether this run wrote {root}/README.md. */
  readme_written: boolean;
  base: IKnowledgeBase;
}

/** A registered knowledge base — a directory of markdown documents. */
export interface IKnowledgeBase {
  knowledge_base_id: KnowledgeBaseId;
  name: string;
  description: string;
  root_path: string;
  /** true = directory provisioned under the backend data dir (purge allowed); false = user-referenced external dir. */
  managed: boolean;
  /** Server-enforced mutation capability chosen explicitly by the local-folder flow. */
  tree_access: 'read_only' | 'editable';
  created_at: number;
  updated_at: number;
  file_count: number;
  total_size: number;
  /** false when the registered root directory no longer exists on disk. */
  root_exists: boolean;
  /** Create-response-only: per-entry fetch summary when the create carried a snapshot-mode URL source. */
  source_fetch?: IKnowledgeSourceFetchSummary;
  /** URL source config when the base has one (top-level on the wire). */
  source?: IKnowledgeSource;
  /** Tag keys attached to this base. */
  tags: string[];
  /** Source kind discriminator. */
  kind: 'blank' | 'local' | 'web';
}

/** A knowledge-base tag (for categorization / filtering). */
export interface IKnowledgeTag {
  key: string;
  label: string;
  color?: string;
  sortOrder: number;
}

/** A single search hit from cross-base semantic/keyword search. */
export interface IKnowledgeSearchHit {
  kb_id: KnowledgeBaseId;
  kb_name: string;
  rel_path: string;
  heading: string;
  snippet: string;
  score: number;
}

type WithProviderEntityId<T> = T extends { provider_id: string }
  ? Omit<T, 'provider_id'> & { provider_id: ProviderId }
  : T;

/** Install-wide, task-exact retrieval pipeline returned by the knowledge API. */
export type IKnowledgeRetrievalConfig = {
  embedding: WithProviderEntityId<ApiKnowledgeRetrievalConfig['embedding']>;
  rerank: WithProviderEntityId<ApiKnowledgeRetrievalConfig['rerank']>;
};

export interface IKnowledgeFileEntry {
  entry_id?: KnowledgeEntryId;
  revision?: number;
  parent_entry_id?: KnowledgeEntryId;
  origin?: 'user' | 'url_snapshot' | 'generated';
  capabilities?: IKnowledgeEntryCapabilities;
  source?: IKnowledgeEntrySource;
  rel_path: string;
  size: number;
  modified_at: number | null;
}

export interface IKnowledgeTreeEntry {
  /** Stable identity projection; optional while legacy/path-only bases are reconciled. */
  entry_id?: KnowledgeEntryId;
  /** Optimistic-concurrency revision of the projected entry. */
  revision?: number;
  parent_entry_id?: KnowledgeEntryId;
  origin?: 'user' | 'url_snapshot' | 'generated';
  capabilities?: IKnowledgeEntryCapabilities;
  source?: IKnowledgeEntrySource;
  name: string;
  rel_path: string;
  is_dir: boolean;
  is_file: boolean;
  size?: number;
  modified_at: number | null;
  children?: IKnowledgeTreeEntry[];
}

export type IKnowledgeRelocateRequest = Omit<
  ApiRelocateKnowledgeEntryRequest,
  'entry_id' | 'destination_parent_id'
> & {
  knowledge_base_id: KnowledgeBaseId;
  /** Stable identity is authoritative; source_path remains the path-only fallback. */
  entry_id?: KnowledgeEntryId;
  /** Stable destination identity is authoritative; its path remains the fallback. */
  destination_parent_id?: KnowledgeEntryId;
};

export type IKnowledgeRelocateResult = Omit<
  ApiRelocateKnowledgeEntryResponse,
  'entry_id'
> & {
  entry_id?: KnowledgeEntryId;
};

export interface IKnowledgeTreeChangedEvent {
  knowledge_base_id: KnowledgeBaseId;
  operation_id: string;
  entry_id?: KnowledgeEntryId;
  old_prefix: string;
  new_prefix: string;
  kind?: 'file' | 'directory';
  moved_descendant_count?: number;
  tree_revision: number;
}

export interface IKnowledgeEntryContentUpdatedEvent {
  knowledge_base_id: KnowledgeBaseId;
  entry_id: KnowledgeEntryId;
  rel_path: string;
  revision?: number;
}

export interface IKnowledgeFileContent {
  rel_path: string;
  content: string;
  size: number;
  modified_at: number | null;
  entry_id?: KnowledgeEntryId;
  revision?: number;
  origin?: 'user' | 'url_snapshot' | 'generated';
  capabilities?: IKnowledgeEntryCapabilities;
  source?: IKnowledgeEntrySource;
}

export interface IKnowledgeFileUpdateResult {
  /** Authoritative current locator; may differ when entry_id followed a move. */
  rel_path: string;
  entry_id?: KnowledgeEntryId;
}

/** Unified receipt for source-management actions on one stable entry. */
export interface IKnowledgeEntrySourceActionResult {
  entry?: IKnowledgeTreeEntry;
  removed?: boolean;
  source_fetch?: IKnowledgeSourceFetchSummary;
}

/** Per-target mount binding: which bases a session mounts + the write-back switch. */
export interface IKnowledgeBinding {
  enabled: boolean;
  writeback: boolean;
  /**
   * Write-back disposition ("回写意识"). 'manual' (default) writes back ONLY
   * when the user explicitly asks — the turn-final extractor is not scheduled
   * at all; 'auto' lets the agent decide on its own against a high bar
   * (durable, reusable, clearly correct — no trivia, no transient state, no
   * duplicates).
   */
  writeback_eagerness: KnowledgeWritebackEagerness;
  /**
   * Opt-in switch letting an unattended IM-channel (bot) session write back to
   * the base. Off by default. Set by the gateway/MCP path (the bot), not the
   * in-app control — but it MUST round-trip through `setBinding` so an in-app
   * edit (toggling bases / write-back) never silently clears it.
   */
  channel_write_enabled: boolean;
  kb_ids: KnowledgeBaseId[];
}

export type KnowledgeWritebackEagerness = 'manual' | 'auto';

export type KnowledgeBindingKind = 'conversation' | 'terminal' | 'companion' | 'workpath';
export type KnowledgeBindingTarget =
  | { kind: 'conversation'; target_id: ConversationId }
  | { kind: 'terminal'; target_id: TerminalId }
  | { kind: 'companion'; target_id: CompanionId }
  | { kind: 'workpath'; target_id: string };

/** Untrusted polymorphic target accepted only at the HTTP adapter boundary. */
type KnowledgeBindingTargetInput = {
  kind: KnowledgeBindingKind;
  target_id: unknown;
};

/** A consumer (binding) of a base — a workspace/conversation/etc. that mounts it. */
export interface IKnowledgeConsumer {
  target_kind: KnowledgeBindingKind | string;
  target_id?: string | null;
  enabled: boolean;
}

// ---------------------------------------------------------------------------
// 客服 (Customer Service) — a standalone domain that safely serves STRANGERS
// over IM channels. Replies come from disposable one-shot engine sessions whose
// tool table is fixed at construction to three read-only tools; dialogues are
// the domain's own aggregate (never Conversations, never the sidebar).
//
// Routed to /api/customer-service (hand-defined against the pinned backend contract).
// ---------------------------------------------------------------------------

/** One customer-service agent (客服员工). */
export interface ICsAgent {
  cs_agent_id: CsAgentId;
  name: string;
  /** 问候语 shown when a visitor opens a conversation. */
  greeting: string;
  /** 人设话术 — persona/voice guidance the agent must follow. */
  persona: string;
  /** 服务策略 — business scope / off-limits topics / compliance phrasing. */
  service_policy: string;
  provider_id: ProviderId | null;
  model: string | null;
  /** Platform knowledge-base ids this agent may retrieve from. */
  knowledge_base_ids: KnowledgeBaseId[];
  enabled: boolean;
  /** Per-agent concurrent turn ceiling (1..=64). */
  max_concurrent: number;
  audit_retention_days: number;
  created_at: number;
  updated_at: number;
}

/** Editable fields on a customer-service agent (PATCH is a partial merge). */
export type ICsAgentPatch = Partial<{
  name: string;
  greeting: string;
  persona: string;
  service_policy: string;
  provider_id: ProviderId | null;
  model: string | null;
  knowledge_base_ids: KnowledgeBaseId[];
  enabled: boolean;
  max_concurrent: number;
  audit_retention_days: number;
}>;

/** One bot ↔ customer-service agent binding (a bot serves at most one agent). */
export interface ICsChannelBinding {
  cs_agent_id: CsAgentId;
  channel_plugin_id: ChannelPluginId;
  created_at: number;
}

/** One customer-service note (FAQ / script / business fact; read-only at runtime). */
export interface ICsNote {
  cs_note_id: CsNoteId;
  /** null = shared by every agent. */
  cs_agent_id: CsAgentId | null;
  kind: string;
  content: string;
  /**
   * Alternate phrasings visitors use for this question, newline separated.
   *
   * Keyword search cannot bridge a paraphrase that shares no words with the
   * note, so these are the operator's way to make such phrasings findable.
   */
  aliases: string;
  enabled: boolean;
  created_at: number;
  updated_at: number;
}

/** One visitor dialogue lane ((bot, visitor, chat) triple). */
export interface ICsDialogue {
  cs_dialogue_id: CsDialogueId;
  cs_agent_id: CsAgentId;
  channel_plugin_id: ChannelPluginId;
  channel_user_id: ChannelUserId;
  chat_id: string;
  state: 'open' | 'closed';
  created_at: number;
  last_activity: number;
}

/** One transcript row of a customer-service dialogue. */
export interface ICsMessage {
  cs_message_id: CsMessageId;
  cs_dialogue_id: CsDialogueId;
  role: 'visitor' | 'agent' | 'system';
  content: string;
  created_at: number;
}

const fromApiCsAgent = (raw: unknown): ICsAgent => {
  const agent = asWireObject(raw, 'customer-service agent');
  if (Object.prototype.hasOwnProperty.call(agent, 'id')) {
    throw new TypeError('customer-service agent wire payload must use cs_agent_id, not id');
  }
  const rawKbIds = agent.knowledge_base_ids;
  const kbIds: KnowledgeBaseId[] = typeof rawKbIds === 'string'
    ? (JSON.parse(rawKbIds) as unknown[]).map(parseKnowledgeBaseId)
    : ((rawKbIds ?? []) as unknown[]).map(parseKnowledgeBaseId);
  return {
    ...(agent as unknown as ICsAgent),
    cs_agent_id: parseCsAgentId(agent.cs_agent_id),
    provider_id: agent.provider_id == null ? null : parseProviderId(agent.provider_id),
    knowledge_base_ids: kbIds,
  };
};

const fromApiCsNote = (raw: unknown): ICsNote => {
  const note = asWireObject(raw, 'customer-service note');
  return {
    ...(note as unknown as ICsNote),
    cs_note_id: parseCsNoteId(note.cs_note_id),
    cs_agent_id: note.cs_agent_id == null ? null : parseCsAgentId(note.cs_agent_id),
  };
};

const fromApiCsBinding = (raw: unknown): ICsChannelBinding => {
  const binding = asWireObject(raw, 'customer-service binding');
  return {
    ...(binding as unknown as ICsChannelBinding),
    cs_agent_id: parseCsAgentId(binding.cs_agent_id),
    channel_plugin_id: parseChannelPluginId(binding.channel_plugin_id),
  };
};

const fromApiCsDialogue = (raw: unknown): ICsDialogue => {
  const dialogue = asWireObject(raw, 'customer-service dialogue');
  return {
    ...(dialogue as unknown as ICsDialogue),
    cs_dialogue_id: parseCsDialogueId(dialogue.cs_dialogue_id),
    cs_agent_id: parseCsAgentId(dialogue.cs_agent_id),
    channel_plugin_id: parseChannelPluginId(dialogue.channel_plugin_id),
    channel_user_id: parseChannelUserId(dialogue.channel_user_id),
  };
};

const fromApiCsMessage = (raw: unknown): ICsMessage => {
  const message = asWireObject(raw, 'customer-service message');
  return {
    ...(message as unknown as ICsMessage),
    cs_message_id: parseCsMessageId(message.cs_message_id),
    cs_dialogue_id: parseCsDialogueId(message.cs_dialogue_id),
  };
};

export const customerService = {
  /** Roster of customer-service agents. */
  listAgents: withResponseMap(
    httpGet<ICsAgent[], void>('/api/customer-service/agents'),
    (agents) => agents.map(fromApiCsAgent)
  ),
  /** Create an agent (name required; everything else defaults server-side). */
  createAgent: withResponseMap(
    httpPost<ICsAgent, Partial<ICsAgentPatch> & { name: string }>('/api/customer-service/agents'),
    fromApiCsAgent
  ),
  /** One agent by cs_agent_id. */
  getAgent: withResponseMap(
    httpGet<ICsAgent, { cs_agent_id: CsAgentId }>(
      (p) => `/api/customer-service/agents/${p.cs_agent_id}`
    ),
    fromApiCsAgent
  ),
  /** Partial merge over the editable fields. Returns the updated agent. */
  patchAgent: withResponseMap(
    httpPatch<ICsAgent, { cs_agent_id: CsAgentId; patch: ICsAgentPatch }>(
      (p) => `/api/customer-service/agents/${p.cs_agent_id}`,
      (p) => p.patch
    ),
    fromApiCsAgent
  ),
  /** Delete an agent (cascades bindings/dialogues/private notes). */
  removeAgent: httpDelete<unknown, { cs_agent_id: CsAgentId }>(
    (p) => `/api/customer-service/agents/${p.cs_agent_id}`
  ),
  /** Bindings of one agent. */
  listBindings: withResponseMap(
    httpGet<ICsChannelBinding[], { cs_agent_id: CsAgentId }>(
      (p) => `/api/customer-service/agents/${p.cs_agent_id}/bindings`
    ),
    (bindings) => bindings.map(fromApiCsBinding)
  ),
  /** FULL replacement of one agent's bot bindings (a listed bot is stolen from any other agent). */
  replaceBindings: withResponseMap(
    httpPut<ICsChannelBinding[], { cs_agent_id: CsAgentId; channel_plugin_ids: ChannelPluginId[] }>(
      (p) => `/api/customer-service/agents/${p.cs_agent_id}/bindings`,
      (p) => ({ channel_plugin_ids: p.channel_plugin_ids })
    ),
    (bindings) => bindings.map(fromApiCsBinding)
  ),
  /** Notes visible to one agent (shared + private), or all when omitted. */
  listNotes: withResponseMap(
    httpGet<ICsNote[], { cs_agent_id?: CsAgentId }>(
      (p) => p.cs_agent_id
        ? `/api/customer-service/notes?cs_agent_id=${p.cs_agent_id}`
        : '/api/customer-service/notes'
    ),
    (notes) => notes.map(fromApiCsNote)
  ),
  createNote: withResponseMap(
    httpPost<ICsNote, { cs_agent_id?: CsAgentId | null; kind?: string; content: string; aliases?: string; enabled?: boolean }>(
      '/api/customer-service/notes'
    ),
    fromApiCsNote
  ),
  patchNote: withResponseMap(
    httpPatch<ICsNote, { cs_note_id: CsNoteId; kind?: string; content?: string; aliases?: string; enabled?: boolean }>(
      (p) => `/api/customer-service/notes/${p.cs_note_id}`,
      (p) => ({ kind: p.kind, content: p.content, aliases: p.aliases, enabled: p.enabled })
    ),
    fromApiCsNote
  ),
  removeNote: httpDelete<unknown, { cs_note_id: CsNoteId }>(
    (p) => `/api/customer-service/notes/${p.cs_note_id}`
  ),
  /** Dialogue lanes of one agent (newest activity first). */
  listDialogues: withResponseMap(
    httpGet<ICsDialogue[], { cs_agent_id: CsAgentId }>(
      (p) => `/api/customer-service/dialogues?cs_agent_id=${p.cs_agent_id}`
    ),
    (dialogues) => dialogues.map(fromApiCsDialogue)
  ),
  /** Full transcript of one dialogue (chronological). */
  listDialogueMessages: withResponseMap(
    httpGet<ICsMessage[], { cs_dialogue_id: CsDialogueId }>(
      (p) => `/api/customer-service/dialogues/${p.cs_dialogue_id}/messages`
    ),
    (messages) => messages.map(fromApiCsMessage)
  ),
};

/**
 * Client-side deadline for knowledge-base READ endpoints. The backend now
 * bounds each base's directory walk (≈6s) and parallelizes the list, so these
 * return quickly in normal operation; this is only a safety net so a wedged
 * NAS/offline root surfaces a legible timeout error instead of hanging the UI.
 * NOT applied to knowledge mutations (autogen / snapshot fetch / import) — those
 * legitimately take minutes.
 */
const KB_READ_TIMEOUT_MS = 30_000;

const fromApiKnowledgeEntrySource = (
  source: IKnowledgeEntrySource
): IKnowledgeEntrySource => ({
  ...source,
  source_id: parseKnowledgeSourceId(source.source_id),
  source_item_id: parseKnowledgeSourceItemId(source.source_item_id),
});

const fromApiKnowledgeBase = (base: IKnowledgeBase): IKnowledgeBase => ({
  ...base,
  knowledge_base_id: parseKnowledgeBaseId(base.knowledge_base_id),
  source: base.source
    ? {
        ...base.source,
        sourceId:
          base.source.sourceId == null
            ? undefined
            : parseKnowledgeSourceId(base.source.sourceId),
        defaultParentEntryId:
          base.source.defaultParentEntryId == null
            ? undefined
            : parseKnowledgeEntryId(base.source.defaultParentEntryId),
        entries: base.source.entries.map((entry) => ({
          ...entry,
          sourceItemId:
            entry.sourceItemId == null
              ? undefined
              : parseKnowledgeSourceItemId(entry.sourceItemId),
          snapshotEntryId:
            entry.snapshotEntryId == null
              ? undefined
              : parseKnowledgeEntryId(entry.snapshotEntryId),
        })),
      }
    : undefined,
});

const fromApiKnowledgeTreeEntry = (entry: IKnowledgeTreeEntry): IKnowledgeTreeEntry => ({
  ...entry,
  entry_id: entry.entry_id == null ? undefined : parseKnowledgeEntryId(entry.entry_id),
  parent_entry_id:
    entry.parent_entry_id == null ? undefined : parseKnowledgeEntryId(entry.parent_entry_id),
  source: entry.source ? fromApiKnowledgeEntrySource(entry.source) : undefined,
  children: entry.children?.map(fromApiKnowledgeTreeEntry),
});

const fromApiKnowledgeFileEntry = (entry: IKnowledgeFileEntry): IKnowledgeFileEntry => ({
  ...entry,
  entry_id: entry.entry_id == null ? undefined : parseKnowledgeEntryId(entry.entry_id),
  parent_entry_id:
    entry.parent_entry_id == null ? undefined : parseKnowledgeEntryId(entry.parent_entry_id),
  source: entry.source ? fromApiKnowledgeEntrySource(entry.source) : undefined,
});

const fromApiKnowledgeFileContent = (file: IKnowledgeFileContent): IKnowledgeFileContent => ({
  ...file,
  entry_id: file.entry_id == null ? undefined : parseKnowledgeEntryId(file.entry_id),
  source: file.source ? fromApiKnowledgeEntrySource(file.source) : undefined,
});

const fromApiKnowledgeEntrySourceActionResult = (
  result: IKnowledgeEntrySourceActionResult
): IKnowledgeEntrySourceActionResult => ({
  ...result,
  entry: result.entry ? fromApiKnowledgeTreeEntry(result.entry) : undefined,
});

const fromApiKnowledgeRelocateResult = (
  result: ApiRelocateKnowledgeEntryResponse
): IKnowledgeRelocateResult => ({
  ...result,
  entry_id: result.entry_id == null ? undefined : parseKnowledgeEntryId(result.entry_id),
});

const fromApiKnowledgeRetrievalStage = <T extends { mode: string }>(
  stage: T
): WithProviderEntityId<T> =>
  (stage.mode === 'remote'
    ? { ...stage, provider_id: parseProviderId((stage as T & { provider_id: unknown }).provider_id) }
    : stage) as WithProviderEntityId<T>;

const fromApiKnowledgeRetrievalConfig = (
  config: ApiKnowledgeRetrievalConfig
): IKnowledgeRetrievalConfig => ({
  embedding: fromApiKnowledgeRetrievalStage(config.embedding),
  rerank: fromApiKnowledgeRetrievalStage(config.rerank),
});

const fromApiKnowledgeBinding = (binding: IKnowledgeBinding): IKnowledgeBinding => ({
  ...binding,
  kb_ids: binding.kb_ids.map(parseKnowledgeBaseId),
});

const parseKnowledgeBindingTargetId = (
  kind: KnowledgeBindingKind,
  value: unknown
): string | ConversationId | TerminalId | CompanionId => {
  if (kind === 'conversation') return parseConversationId(value);
  if (kind === 'terminal') return parseTerminalId(value);
  if (kind === 'companion') return parseCompanionId(value);
  if (typeof value !== 'string' || value.length === 0 || value.trim() !== value) {
    throw new TypeError('workpath binding target must be a non-empty canonical path');
  }
  return value;
};

const parseKnowledgeBindingTarget = (
  target: KnowledgeBindingTargetInput
): KnowledgeBindingTarget => {
  if (target.kind === 'conversation') {
    return { kind: target.kind, target_id: parseConversationId(target.target_id) };
  }
  if (target.kind === 'terminal') {
    return { kind: target.kind, target_id: parseTerminalId(target.target_id) };
  }
  if (target.kind === 'companion') {
    return { kind: target.kind, target_id: parseCompanionId(target.target_id) };
  }
  return {
    kind: target.kind,
    target_id: parseKnowledgeBindingTargetId(target.kind, target.target_id),
  };
};

export const knowledge = {
  listBases: withResponseMap(httpGet<IKnowledgeBase[], void>('/api/knowledge/bases', {
    timeoutMs: KB_READ_TIMEOUT_MS,
  }), (bases) => bases.map(fromApiKnowledgeBase)),
  createBase: withResponseMap(httpPost<
    IKnowledgeBase,
    {
      name: string;
      description?: string;
      root_path?: string;
      tree_access?: 'read_only' | 'editable';
      /** Optional URL source; mode 'snapshot' fetches every entry before the response returns (slow — see source_fetch). */
      source?: { kind: string; mode: KnowledgeSourceMode; entries?: IKnowledgeSourceEntry[] };
      /** Tag keys to assign at creation time. */
      tags?: string[];
    }
  >('/api/knowledge/bases'), fromApiKnowledgeBase),
  getBase: withResponseMap(httpGet<IKnowledgeBase, { knowledge_base_id: KnowledgeBaseId }>((p) => `/api/knowledge/bases/${p.knowledge_base_id}`, { timeoutMs: KB_READ_TIMEOUT_MS }), fromApiKnowledgeBase),
  updateBase: withResponseMap(httpPut<IKnowledgeBase, { knowledge_base_id: KnowledgeBaseId; name?: string; description?: string; tags?: string[]; tree_access?: 'read_only' | 'editable' }>(
    (p) => `/api/knowledge/bases/${p.knowledge_base_id}`,
    (p) => ({ name: p.name, description: p.description, tags: p.tags, tree_access: p.tree_access })
  ), fromApiKnowledgeBase),
  getRetrievalConfig: withResponseMap(
    httpGet<ApiKnowledgeRetrievalConfig, void>('/api/knowledge/retrieval'),
    fromApiKnowledgeRetrievalConfig
  ),
  setRetrievalConfig: withResponseMap(
    httpPut<ApiKnowledgeRetrievalConfig, IKnowledgeRetrievalConfig>('/api/knowledge/retrieval'),
    fromApiKnowledgeRetrievalConfig
  ),
  /** AI overview generation (description + README.md). Slow (LLM round-trip, 30s+); 409 when no AI provider is configured. */
  autogenBase: withResponseMap(httpPost<IKnowledgeAutogenOutcome, { knowledge_base_id: KnowledgeBaseId; overwrite_readme?: boolean; provider_id?: ProviderId; model?: string }>(
    (p) => `/api/knowledge/bases/${p.knowledge_base_id}/autogen`,
    (p) => ({
      overwrite_readme: p.overwrite_readme ?? false,
      provider_id: p.provider_id,
      model: p.model,
    })
  ), (outcome) => ({ ...outcome, base: fromApiKnowledgeBase(outcome.base) })),
  /**
   * Stateless AI description draft from a local directory (no base required — used by the create form).
   * Slow (LLM round-trip); 409 when no AI completer is configured, 400 when the path is invalid.
   */
  generateDescription: httpPost<{ description: string }, { name?: string; root_path: string; provider_id?: ProviderId; model?: string }>(
    '/api/knowledge/description/generate',
    (p) => ({ name: p.name, root_path: p.root_path, provider_id: p.provider_id, model: p.model })
  ),
  /** Stateless AI polish of a hand-written description draft. Slow (LLM round-trip); 409 when no AI completer is configured. */
  polishDescription: httpPost<{ description: string }, { name?: string; draft: string; provider_id?: ProviderId; model?: string }>(
    '/api/knowledge/description/polish',
    (p) => ({ name: p.name, draft: p.draft, provider_id: p.provider_id, model: p.model })
  ),
  /** Re-fetch every URL-source entry at its current identity-backed location. */
  refreshSource: httpPost<IKnowledgeSourceFetchSummary, { knowledge_base_id: KnowledgeBaseId }>(
    (p) => `/api/knowledge/bases/${p.knowledge_base_id}/refresh-source`,
    () => undefined
  ),
  /** Attach / replace / clear a base's source config. */
  setSource: withResponseMap(httpPut<IKnowledgeBase, { knowledge_base_id: KnowledgeBaseId; source: IKnowledgeSource | null }>(
    (p) => `/api/knowledge/bases/${p.knowledge_base_id}/source`,
    (p) => ({ source: p.source })
  ), fromApiKnowledgeBase),
  deleteBase: httpDelete<void, { knowledge_base_id: KnowledgeBaseId; purge?: boolean }>(
    (p) => `/api/knowledge/bases/${p.knowledge_base_id}${p.purge ? '?purge=true' : ''}`
  ),
  listFiles: withResponseMap(
    httpGet<IKnowledgeFileEntry[], { knowledge_base_id: KnowledgeBaseId }>(
      (p) => `/api/knowledge/bases/${p.knowledge_base_id}/files`,
      { timeoutMs: KB_READ_TIMEOUT_MS }
    ),
    (entries) => entries.map(fromApiKnowledgeFileEntry)
  ),
  /** Add a new document, copied Markdown folder, or managed web entries to an existing base. */
  addContent: httpPost<IKnowledgeAddContentResult, IKnowledgeAddContentRequest>(
    (p) => `/api/knowledge/bases/${p.knowledge_base_id}/content`,
    (p) => {
      const { knowledge_base_id: _knowledgeBaseId, ...body } = p;
      return body;
    }
  ),
  listTree: withResponseMap(
    httpGet<IKnowledgeTreeEntry[], { knowledge_base_id: KnowledgeBaseId; path?: string }>(
      (p) => `/api/knowledge/bases/${p.knowledge_base_id}/tree${p.path ? `?path=${encodeURIComponent(p.path)}` : ''}`,
      { timeoutMs: KB_READ_TIMEOUT_MS }
    ),
    (entries) => entries.map(fromApiKnowledgeTreeEntry)
  ),
  createFolder: httpPost<IKnowledgeTreeEntry, { knowledge_base_id: KnowledgeBaseId; path: string }>(
    (p) => `/api/knowledge/bases/${p.knowledge_base_id}/folder`,
    (p) => ({ path: p.path })
  ),
  deleteFolder: httpDelete<
    void,
    {
      knowledge_base_id: KnowledgeBaseId;
      path: string;
      entry_id?: KnowledgeEntryId;
      expected_revision?: number;
    }
  >(
    (p) =>
      `/api/knowledge/bases/${p.knowledge_base_id}/folder?path=${encodeURIComponent(p.path)}` +
      (p.entry_id && p.expected_revision != null
        ? `&entry_id=${encodeURIComponent(p.entry_id)}&expected_revision=${p.expected_revision}`
        : '')
  ),
  renameTreeEntry: httpPost<IKnowledgeTreeEntry, { knowledge_base_id: KnowledgeBaseId; path: string; newName: string }>(
    (p) => `/api/knowledge/bases/${p.knowledge_base_id}/tree/rename`,
    (p) => ({ path: p.path, new_name: p.newName })
  ),
  /** Move or rename one existing file/directory without overwriting the destination. */
  relocateTreeEntry: withResponseMap(
    httpPost<ApiRelocateKnowledgeEntryResponse, IKnowledgeRelocateRequest>(
      (p) => `/api/knowledge/bases/${p.knowledge_base_id}/tree/relocate`,
      (p) => {
        const { knowledge_base_id: _knowledgeBaseId, ...body } = p;
        return body;
      }
    ),
    fromApiKnowledgeRelocateResult
  ),
  undoRelocateTreeEntry: withResponseMap(
    httpPost<
      ApiRelocateKnowledgeEntryResponse,
      { knowledge_base_id: KnowledgeBaseId; request_id: string; undo_token: string }
    >(
      (p) => `/api/knowledge/bases/${p.knowledge_base_id}/tree/relocate/undo`,
      (p) => ({ request_id: p.request_id, undo_token: p.undo_token })
    ),
    fromApiKnowledgeRelocateResult
  ),
  refreshEntrySource: withResponseMap(
    httpPost<
      IKnowledgeEntrySourceActionResult,
      {
        knowledge_base_id: KnowledgeBaseId;
        entry_id: KnowledgeEntryId;
        expected_revision?: number;
      }
    >(
      (p) =>
        `/api/knowledge/bases/${p.knowledge_base_id}/entries/${p.entry_id}/refresh-source`,
      (p) => ({ expected_revision: p.expected_revision })
    ),
    fromApiKnowledgeEntrySourceActionResult
  ),
  copyEntryAsEditable: withResponseMap(
    httpPost<
      IKnowledgeEntrySourceActionResult,
      {
        knowledge_base_id: KnowledgeBaseId;
        entry_id: KnowledgeEntryId;
        expected_revision?: number;
        destination_parent_path?: string;
        destination_parent_id?: KnowledgeEntryId;
        new_name?: string;
      }
    >(
      (p) =>
        `/api/knowledge/bases/${p.knowledge_base_id}/entries/${p.entry_id}/copy-as-editable`,
      (p) => ({
        expected_revision: p.expected_revision,
        destination_parent_path: p.destination_parent_path,
        destination_parent_id: p.destination_parent_id,
        new_name: p.new_name,
      })
    ),
    fromApiKnowledgeEntrySourceActionResult
  ),
  detachEntrySource: withResponseMap(
    httpPost<
      IKnowledgeEntrySourceActionResult,
      {
        knowledge_base_id: KnowledgeBaseId;
        entry_id: KnowledgeEntryId;
        expected_revision?: number;
      }
    >(
      (p) =>
        `/api/knowledge/bases/${p.knowledge_base_id}/entries/${p.entry_id}/detach-source`,
      (p) => ({ expected_revision: p.expected_revision })
    ),
    fromApiKnowledgeEntrySourceActionResult
  ),
  removeEntrySource: withResponseMap(
    httpPost<
      IKnowledgeEntrySourceActionResult,
      {
        knowledge_base_id: KnowledgeBaseId;
        entry_id: KnowledgeEntryId;
        expected_revision?: number;
      }
    >(
      (p) =>
        `/api/knowledge/bases/${p.knowledge_base_id}/entries/${p.entry_id}/remove-source`,
      (p) => ({ expected_revision: p.expected_revision })
    ),
    fromApiKnowledgeEntrySourceActionResult
  ),
  readFile: withResponseMap(
    httpGet<IKnowledgeFileContent, { knowledge_base_id: KnowledgeBaseId; path: string }>(
      (p) => `/api/knowledge/bases/${p.knowledge_base_id}/file?path=${encodeURIComponent(p.path)}`,
      { timeoutMs: KB_READ_TIMEOUT_MS }
    ),
    fromApiKnowledgeFileContent
  ),
  writeFile: withResponseMap(
    httpPut<
      IKnowledgeFileUpdateResult,
      {
        knowledge_base_id: KnowledgeBaseId;
        path: string;
        content: string;
        expected_content?: string;
        entry_id?: KnowledgeEntryId;
        expected_revision?: number;
      }
    >(
      (p) => `/api/knowledge/bases/${p.knowledge_base_id}/file`,
      (p) => ({
        path: p.path,
        content: p.content,
        expected_content: p.expected_content,
        entry_id: p.entry_id,
        expected_revision: p.expected_revision,
      })
    ),
    (result) => ({
      ...result,
      entry_id: result.entry_id == null ? undefined : parseKnowledgeEntryId(result.entry_id),
    })
  ),
  deleteFile: httpDelete<
    void,
    {
      knowledge_base_id: KnowledgeBaseId;
      path: string;
      entry_id?: KnowledgeEntryId;
      expected_revision?: number;
    }
  >(
    (p) =>
      `/api/knowledge/bases/${p.knowledge_base_id}/file?path=${encodeURIComponent(p.path)}` +
      (p.entry_id && p.expected_revision != null
        ? `&entry_id=${encodeURIComponent(p.entry_id)}&expected_revision=${p.expected_revision}`
        : '')
  ),
  getBinding: withResponseMap(httpGet<IKnowledgeBinding, KnowledgeBindingTargetInput>(
    // workpath target_id is a filesystem path containing `/`; encode so it
    // stays a single path segment (`/`→`%2F`). conversation/terminal ids have
    // no `/`, so their encoded form is byte-identical — no regression.
    (p) => {
      const target = parseKnowledgeBindingTarget(p);
      return `/api/knowledge/binding/${target.kind}/${encodeURIComponent(target.target_id)}`;
    }
  ), fromApiKnowledgeBinding),
  setBinding: withResponseMap(httpPost<IKnowledgeBinding, KnowledgeBindingTargetInput & IKnowledgeBinding>(
    (p) => {
      const target = parseKnowledgeBindingTarget(p);
      return `/api/knowledge/binding/${target.kind}/${encodeURIComponent(target.target_id)}`;
    },
    // Forward EVERY binding field by destructuring off the routing params only.
    // A hand-maintained whitelist here silently dropped writeback_eagerness and
    // channel_write_enabled in turn (the backend POST is a full replace), so any
    // new IKnowledgeBinding field stays in the body automatically.
    (p) => {
      const { kind: _kind, target_id: _target_id, ...body } = p;
      return body;
    }
  ), fromApiKnowledgeBinding),
  // ── Import / export (spec 2026-06-11 §4.8: zip with manifest.kind="knowledge-base") ──
  exportBase: httpPost<{ dest_path: string }, { knowledge_base_id: KnowledgeBaseId; dest_path: string }>(
    (p) => `/api/knowledge/bases/${p.knowledge_base_id}/export`,
    (p) => ({ dest_path: p.dest_path })
  ),
  /** Import a knowledge-base bundle — a new managed base is provisioned (name conflicts get a "(2)" suffix). */
  importBase: withResponseMap(httpPost<IKnowledgeBase, { src_path: string }>('/api/knowledge/bases/import'), fromApiKnowledgeBase),
  /** Bindings currently mounting this base (enabled AND disabled). */
  listConsumers: httpGet<IKnowledgeConsumer[], { knowledge_base_id: KnowledgeBaseId }>((p) => `/api/knowledge/bases/${p.knowledge_base_id}/consumers`, { timeoutMs: KB_READ_TIMEOUT_MS }),
  onBaseCreated: wsMappedEmitter<IKnowledgeBase>('knowledge.base-created', fromApiKnowledgeBase),
  onBaseUpdated: wsMappedEmitter<IKnowledgeBase>('knowledge.base-updated', fromApiKnowledgeBase),
  onBaseDeleted: wsMappedEmitter<{ knowledge_base_id: KnowledgeBaseId }>(
    'knowledge.base-deleted',
    (value) => ({ knowledge_base_id: parseKnowledgeBaseId(value.knowledge_base_id) })
  ),
  onTreeChanged: wsMappedEmitter<
    IKnowledgeTreeChangedEvent,
    Omit<IKnowledgeTreeChangedEvent, 'knowledge_base_id'> & {
      knowledge_base_id: string;
      old_path?: string;
      new_path?: string;
      old_prefix?: string;
      new_prefix?: string;
    }
  >('knowledge.tree-changed', (value) => ({
    knowledge_base_id: parseKnowledgeBaseId(value.knowledge_base_id),
    operation_id: value.operation_id,
    entry_id: value.entry_id == null ? undefined : parseKnowledgeEntryId(value.entry_id),
    old_prefix: value.old_prefix ?? value.old_path ?? '',
    new_prefix: value.new_prefix ?? value.new_path ?? '',
    kind: value.kind,
    moved_descendant_count: value.moved_descendant_count,
    tree_revision: value.tree_revision,
  })),
  onEntryContentUpdated: wsMappedEmitter<
    IKnowledgeEntryContentUpdatedEvent,
    Omit<IKnowledgeEntryContentUpdatedEvent, 'knowledge_base_id' | 'entry_id'> & {
      knowledge_base_id: string;
      entry_id: string;
    }
  >('knowledge.entry-content-updated', (value) => ({
    knowledge_base_id: parseKnowledgeBaseId(value.knowledge_base_id),
    entry_id: parseKnowledgeEntryId(value.entry_id),
    rel_path: value.rel_path,
    revision: value.revision,
  })),
  onBindingChanged: wsMappedEmitter<{ target_kind: KnowledgeBindingKind; target_id: string | ConversationId | TerminalId | CompanionId } & IKnowledgeBinding>(
    'knowledge.binding-changed',
    (value) => ({
      ...fromApiKnowledgeBinding(value),
      target_kind: value.target_kind,
      target_id: parseKnowledgeBindingTargetId(value.target_kind, value.target_id),
    })
  ),
  /** A tag was created/renamed/recolored/reordered/deleted — re-list tags. */
  onTagChanged: wsEmitter<Record<string, never>>('knowledge.tag-changed'),
  // ── Tags (categorization / filtering) ──
  listTags: httpGet<IKnowledgeTag[], void>('/api/knowledge/tags'),
  createTag: httpPost<IKnowledgeTag, { label: string; color?: string }>(
    '/api/knowledge/tags',
    (p) => ({ label: p.label, color: p.color })
  ),
  updateTag: httpPut<void, { key: string; label?: string; color?: string; sortOrder?: number }>(
    (p) => `/api/knowledge/tags/${p.key}`,
    (p) => ({ label: p.label, color: p.color, sortOrder: p.sortOrder })
  ),
  deleteTag: httpDelete<void, { key: string }>((p) => `/api/knowledge/tags/${p.key}`),
  // ── Cross-base search ──
  search: withResponseMap(httpPost<IKnowledgeSearchHit[], { kbIds: KnowledgeBaseId[]; query: string; limit?: number }>(
    '/api/knowledge/search',
    (p) => ({
      kbIds: p.kbIds,
      query: p.query,
      limit: p.limit,
    })
  ), (hits) => hits.map((hit) => ({ ...hit, kb_id: parseKnowledgeBaseId(hit.kb_id) }))),
};
