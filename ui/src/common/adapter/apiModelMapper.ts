/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TProviderWithModel } from '../config/storage';
import {
  parseConversationId,
  parseCronJobId,
  parseExecutionAttemptId,
  parseExecutionId,
  parseExecutionStepId,
  parseExecutionTemplateId,
  parseProviderId,
  parseRemoteAgentId,
} from '../types/ids';

export type ApiProviderWithModel = {
  provider_id: string;
  model: string;
  use_model?: string;
};

function hasCompleteModelIdentity(
  model?: TProviderWithModel
): model is TProviderWithModel & { id: string; use_model: string } {
  return Boolean(
    model &&
    typeof model.id === 'string' &&
    model.id.trim().length > 0 &&
    typeof model.use_model === 'string' &&
    model.use_model.trim().length > 0
  );
}

// ── Frontend → Backend ──────────────────────────────────────────────────

export function toApiModel(m: TProviderWithModel): ApiProviderWithModel {
  return {
    provider_id: m.id,
    model: m.use_model,
  };
}

export function toApiModelOptional(m?: TProviderWithModel): ApiProviderWithModel | undefined {
  return hasCompleteModelIdentity(m) ? toApiModel(m) : undefined;
}

// ── Backend → Frontend ──────────────────────────────────────────────────

export function fromApiModel(raw: ApiProviderWithModel): TProviderWithModel {
  return {
    id: parseProviderId(raw.provider_id),
    platform: '',
    name: '',
    base_url: '',
    api_key: '',
    use_model: raw.use_model ?? raw.model,
  };
}

function fromApiModelOptional(raw?: ApiProviderWithModel | null): TProviderWithModel | undefined {
  return raw ? fromApiModel(raw) : undefined;
}

/** ConversationResponse 顶层置顶字段（conversations 表真列，服务端维护 pinned_at）。 */
export type ApiConversationPinnedFields = {
  pinned?: boolean | null;
  /** 毫秒时间戳；未置顶时服务端省略该 key */
  pinned_at?: number | null;
};

/** First-class Conversation collaboration authoring reference. It is never
 * read from or mirrored into `extra`. */
export type ApiConversationExecutionTemplateFields = {
  execution_template_id?: string | null;
};

export function fromApiConversation<T>(raw: T): T {
  if (!raw || typeof raw !== 'object') return raw;

  const r = raw as T &
    ApiConversationPinnedFields &
    ApiConversationExecutionTemplateFields & {
      id?: unknown;
      model?: ApiProviderWithModel | null;
      extra?: Record<string, unknown> | null;
      /** Promoted to a top-level conversations column (was extra.cronJobId). */
      cron_job_id?: string | null;
      execution_template_id?: string | null;
      linked_execution_id?: string | null;
      execution_step_id?: string | null;
      execution_attempt_id?: string | null;
    };
  const next = { ...r } as unknown as T & {
    id?: ReturnType<typeof parseConversationId>;
    model?: TProviderWithModel;
    extra?: Record<string, unknown> | null;
    cron_job_id?: ReturnType<typeof parseCronJobId>;
    execution_template_id?: ReturnType<typeof parseExecutionTemplateId> | null;
    linked_execution_id?: ReturnType<typeof parseExecutionId>;
    execution_step_id?: ReturnType<typeof parseExecutionStepId>;
    execution_attempt_id?: ReturnType<typeof parseExecutionAttemptId>;
  };

  if ('id' in r) {
    next.id = parseConversationId(r.id);
  }

  if ('model' in r) {
    next.model = fromApiModelOptional(r.model);
  }

  if (r.cron_job_id != null) next.cron_job_id = parseCronJobId(r.cron_job_id);
  if (r.execution_template_id != null) {
    next.execution_template_id = parseExecutionTemplateId(r.execution_template_id);
  } else if ('execution_template_id' in r) {
    next.execution_template_id = null;
  }
  if (r.linked_execution_id != null) next.linked_execution_id = parseExecutionId(r.linked_execution_id);
  if (r.execution_step_id != null) next.execution_step_id = parseExecutionStepId(r.execution_step_id);
  if (r.execution_attempt_id != null) next.execution_attempt_id = parseExecutionAttemptId(r.execution_attempt_id);

  let extra = r.extra && typeof r.extra === 'object' ? r.extra : null;

  if (extra && !('custom_workspace' in extra)) {
    const workspace = typeof extra.workspace === 'string' ? extra.workspace : '';
    const isTemporary = extra.is_temporary_workspace === true;
    extra = {
      ...extra,
      custom_workspace: workspace.length > 0 && !isTemporary,
    };
  }

  // Remote-agent conversations use snake_case on the backend. Mirror the row
  // id to the legacy camelCase key while older UI call sites are still being
  // upgraded, so both fresh and existing conversations resolve their agent.
  if (extra && 'remote_agent_id' in extra) {
    const remoteAgentId = parseRemoteAgentId(extra.remote_agent_id);
    extra = {
      ...extra,
      remote_agent_id: remoteAgentId,
      remoteAgentId: remoteAgentId,
    };
  }

  // cron_job_id 镜像：后端把它从 extra.cronJobId 提升为顶层列；为保持既有读路径
  // （多处读 conversation.extra?.cron_job_id）不变，将顶层值镜像回 extra。
  if (next.cron_job_id) {
    extra = { ...(extra ?? {}), cron_job_id: next.cron_job_id };
  }

  // 置顶镜像：DB 顶层 pinned/pinned_at 列为权威，镜像进 extra，客户端读路径
  // 保持单一（isConversationPinned / workpathTree 只读 extra）。
  // 兼容规则：extra.pinned = 列 || extra（列为准，但旧数据仅 extra 置顶的不丢）；
  // 冲突时 pinned_at 取列值（服务端维护），仅 extra 置顶时保留 extra.pinned_at。
  if (r.pinned === true) {
    const base = extra ?? {};
    const pinnedAt = typeof r.pinned_at === 'number' ? r.pinned_at : typeof base.pinned_at === 'number' ? (base.pinned_at as number) : undefined;
    extra = {
      ...base,
      pinned: true,
      ...(pinnedAt !== undefined ? { pinned_at: pinnedAt } : {}),
    };
  }

  if (extra && extra !== r.extra) {
    next.extra = extra;
  }

  return next;
}

export function fromApiPaginatedConversations<T>(result: { items: T[]; total: number; has_more: boolean }): {
  items: T[];
  total: number;
  has_more: boolean;
} {
  return {
    ...result,
    items: result.items.map(fromApiConversation),
  };
}
