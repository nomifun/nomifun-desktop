/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ProviderModelResponse } from '@/common/types/provider/providerModel';
import type { PresetReference, ResolvedPresetSnapshot } from '@/common/types/agent/presetTypes';
import type {
  TDecisionPolicy,
  TDelegationPolicy,
  TExecutionModelPool,
} from '@/common/types/agentExecution/agentExecutionTypes';
import type {
  ConversationId,
  CompanionId,
  CronJobId,
  ExecutionAttemptId,
  ExecutionId,
  ExecutionStepId,
  ExecutionTemplateId,
  MessageId,
  McpServerId,
  ProviderId,
} from '@/common/types/ids';

/**
 * Conversation source type - identifies where the conversation was created
 * 会话来源类型 - 标识会话创建的来源
 */
export type ConversationSource = 'nomifun' | 'telegram' | 'lark' | 'dingtalk' | 'weixin' | 'wecom' | (string & {});

export type TChatConversationStatus = 'pending' | 'running' | 'finished';
export type TConversationRuntimeStateKind = 'idle' | 'starting' | 'running' | 'waiting_confirmation';

export type TConversationRuntimeSummary = {
  state: TConversationRuntimeStateKind;
  can_send_message: boolean;
  has_runtime: boolean;
  runtime_status?: TChatConversationStatus;
  is_processing: boolean;
  pending_confirmations: number;
  /** Exact backend turn currently owning this runtime. This is lifecycle
   * authority; processing_started_at is display-only and may collide. */
  active_turn_id?: MessageId;
  /** Epoch ms when the currently-running turn started, present while
   *  is_processing. Anchors the elapsed-time indicator so it survives view
   *  unmount/remount (tab/session switch) instead of restarting from zero. */
  processing_started_at?: number;
};

interface IChatConversation<T, Extra> {
  created_at: number;
  modified_at: number;
  name: string;
  desc?: string;
  /** Canonical backend-minted Conversation entity id. */
  id: ConversationId;
  type: T;
  extra: Extra;
  model: TProviderWithModel;
  status?: TChatConversationStatus | undefined;
  runtime?: TConversationRuntimeSummary;
  /** 会话来源，默认为 nomifun / Conversation source, defaults to nomifun */
  source?: ConversationSource;
  /** First-class conversation pin state. This is the only authoritative UI field. */
  pinned?: boolean;
  pinned_at?: number;
  /** Channel chat isolation ID (e.g. user:xxx, group:xxx) */
  channel_chat_id?: string;
  /** Cron job that spawned this conversation. */
  cron_job_id?: CronJobId;
  /** Immutable preset lineage resolved and persisted by the backend. */
  preset_id?: PresetReference;
  preset_revision?: number;
  preset_snapshot?: ResolvedPresetSnapshot;
  /** Nomi-only collaboration policy persisted as first-class conversation fields. */
  delegation_policy?: TDelegationPolicy;
  execution_model_pool?: TExecutionModelPool;
  decision_policy?: TDecisionPolicy;
  /** Optional collaboration authoring template. Runtime executions copy its
   * resolved snapshot and never retain this mutable reference. `null` is the
   * PATCH wire value for explicitly clearing the selection. */
  execution_template_id?: ExecutionTemplateId | null;
  /** Collaboration aggregate linked to this lead or retained Attempt transcript. */
  linked_execution_id?: ExecutionId;
  execution_step_id?: ExecutionStepId;
  execution_attempt_id?: ExecutionAttemptId;
}

// Token 使用统计数据类型
export interface TokenUsageData {
  total_tokens: number;
  /** Model input for the completed turn. */
  input_tokens?: number;
  /** Model output for the completed turn. Includes provider-accounted reasoning when applicable. */
  output_tokens?: number;
  /** Provider-reported reasoning subset/detail. Never added to total_tokens a second time. */
  reasoning_tokens?: number;
  /** Current context occupancy (gauge numerator). */
  context_tokens?: number;
  /** Effective context budget (gauge denominator). */
  context_window?: number;
}

export type TChatConversation = IChatConversation<
  'nomi',
  {
    workspace: string;
    custom_workspace?: boolean;
    proxy?: string;
    /** Skills snapshot for this conversation — authoritative list, written
     * once at creation. Join with `GET /api/skills` for descriptions. */
    skills?: string[];
    /** MCP server id snapshot chosen when the conversation was created. */
    mcp_server_ids?: McpServerId[];
    /** MCP server name snapshot chosen when the conversation was created. */
    mcp_servers?: string[];
    /** Conversation-scoped MCP status snapshot shown in the sendbox menu. */
    mcp_statuses?: IConversationMcpStatus[];
    /** Session-only MCP server snapshot persisted at creation time. */
    session_mcp_servers?: ISessionMcpServer[];
    /** Max tokens per response */
    /** Max agentic turns */
    maxTurns?: number;
    /** Persisted session mode for resume support */
    session_mode?: string;
    /** Legacy marker for pre-provider-probe health-check conversations */
    is_health_check?: boolean;
    /** Last token usage stats */
    last_token_usage?: TokenUsageData;
    /** Marks this nomi conversation as a desktop-companion's single per-companion
     * session (单会话契约). Written by the backend at companion-session creation.
     * Drives the 桌面伙伴 session-list group, the constrained companion chat panel
     * (CompanionChatPanel), and the work-conversation list filter. */
    companion_session?: boolean;
    /** The companion (桌面伙伴) this session belongs to, when `companion_session` is
     * set. Resolves the companion profile for the constrained chat panel + the
     * session-list group's active-row highlight. */
    companion_id?: CompanionId;
    /** IM-channel platform when a companion turn originated from an external
     * channel (telegram/lark/…). Present on channel-sourced companion turns. */
    channel_platform?: string;
    /** In-session companion summon marker（设计 B）: the summoned companion's
     * id + hand-picked memory ids + excluded skills, `summoned_at`
     * server-stamped. Written only through PUT
     * /api/conversations/{id}/summon or trusted backend creators; drives
     * the sendbox summon control and the header/sidebar badges. */
    summon?: {
      companion_id: CompanionId;
      memory_ids: string[];
      skill_exclusions: string[];
      summoned_at: number;
    };
  }
>;

export type IChatConversationRefer = {
  'chat.history': TChatConversation[];
};

/**
 * 统一多模态能力词表 —— ts-rs 生成契约的 re-export（生成源
 * crates/backend/nomifun-api-types/src/model_task.rs，由
 * `cargo test -p nomifun-api-types` 重新生成到 @/common/protocolBindings/）。
 * ModelTask 决定端点/请求体；ModelTrait 是同一任务内的细化（主要修饰 chat）。
 */
export type { ModelTask } from '@/common/protocolBindings/ModelTask';
export type { ModelTrait } from '@/common/protocolBindings/ModelTrait';

/** 权威 per-model 能力档案（键 (provider_id, model)）。 */
export interface IProvider {
  id: ProviderId;
  platform: string;
  name: string;
  base_url: string;
  /** Explicit auth transport for the provider's default connection. */
  auth_scheme: string;
  /** Credentials are write-only; responses expose only whether any are configured. */
  has_credentials: boolean;
  /** Authoritative configured models with their complete task capabilities. */
  models: ProviderModelResponse[];
  /**
   * AWS Bedrock specific configuration
   * Only used when platform is 'bedrock'
   */
  bedrock_config?: {
    auth_method: 'accessKey' | 'profile' | 'defaultChain';
    region: string;
    /** Non-secret AWS profile name; present only for profile auth. */
    profile?: string;
  };
  /**
   * 供应商启用状态，默认为 true
   * Provider enabled state, defaults to true
   */
  enabled?: boolean;
  /**
   * 供应商排序优先级，数值越小优先级越高。
   * Provider priority order; lower values are used first.
   */
  sort_order?: number;
}

export type TProviderWithModel = Omit<IProvider, 'models'> & {
  use_model: string;
};

// MCP Server Configuration Types
export interface IMcpServerTransportStdio {
  type: 'stdio';
  command: string;
  args?: string[];
  env?: Record<string, string>;
}

export interface IMcpServerTransportSSE {
  type: 'sse';
  url: string;
  headers?: Record<string, string>;
}

export interface IMcpServerTransportHTTP {
  type: 'http';
  url: string;
  headers?: Record<string, string>;
}

export interface IMcpServerTransportStreamableHTTP {
  type: 'streamable_http';
  url: string;
  headers?: Record<string, string>;
}

export type IMcpServerTransport =
  | IMcpServerTransportStdio
  | IMcpServerTransportSSE
  | IMcpServerTransportHTTP
  | IMcpServerTransportStreamableHTTP;

export interface IMcpServer {
  mcp_server_id: McpServerId;
  name: string;
  description?: string;
  enabled: boolean; // 是否默认启用（新会话默认勾选）
  transport: IMcpServerTransport;
  tools?: IMcpTool[];
  last_test_status?: 'connected' | 'disconnected' | 'error' | 'testing'; // 最近一次检测结果
  last_connected?: number;
  created_at: number;
  updated_at: number;
  original_json: string; // 存储原始JSON配置，用于编辑时的准确显示
  /** Built-in MCP server managed by Nomi (hide edit/delete in UI) */
  builtin?: boolean;
}

/** Conversation-scoped MCP snapshot keyed by the stable MCP business ID. */
export interface ISessionMcpServer {
  mcp_server_id: McpServerId;
  name: string;
  transport: IMcpServerTransport;
}

export type IConversationMcpStatusKind = 'loaded' | 'failed' | 'unsupported';

export interface IConversationMcpStatus {
  mcp_server_id: McpServerId;
  name: string;
  status: IConversationMcpStatusKind;
  reason?: string;
}

export interface IMcpTool {
  name: string;
  description?: string;
  input_schema?: unknown;
  _meta?: Record<string, unknown>;
}

/**
 * CSS 主题配置接口 / CSS Theme configuration interface
 * 用于存储用户自定义的 CSS 皮肤 / Used to store user-defined CSS skins
 */
export interface ICssTheme {
  id: string; // 唯一标识 / Unique identifier
  name: string; // 主题名称 / Theme name
  cover?: string; // 封面图片 base64 或 URL / Cover image base64 or URL
  css: string; // CSS 样式代码 / CSS style code
  is_preset?: boolean; // 是否为预设主题 / Whether it's a preset theme
  created_at: number; // 创建时间 / Creation time
  updated_at: number; // 更新时间 / Update time
}
