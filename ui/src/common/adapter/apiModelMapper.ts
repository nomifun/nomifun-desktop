/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import type { TProviderWithModel } from '../config/storage';

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
    id: raw.provider_id,
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

export function fromApiConversation<T>(raw: T): T {
  if (!raw || typeof raw !== 'object') return raw;

  const r = raw as T &
    ApiConversationPinnedFields & {
      model?: ApiProviderWithModel | null;
      extra?: Record<string, unknown> | null;
      /** Promoted to a top-level conversations column (was extra.cronJobId). */
      cron_job_id?: string | null;
    };
  const next = { ...r } as unknown as T & {
    model?: TProviderWithModel;
    extra?: Record<string, unknown> | null;
  };

  if ('model' in r) {
    next.model = fromApiModelOptional(r.model);
  }

  let extra = r.extra && typeof r.extra === 'object' ? r.extra : null;

  if (extra && !('custom_workspace' in extra)) {
    const workspace = typeof extra.workspace === 'string' ? extra.workspace : '';
    const isTemporary = extra.is_temporary_workspace === true;
    extra = {
      ...extra,
      custom_workspace: workspace.length > 0 && !isTemporary,
    };
  }

  // cron_job_id 镜像：后端把它从 extra.cronJobId 提升为顶层列；为保持既有读路径
  // （多处读 conversation.extra?.cron_job_id）不变，将顶层值镜像回 extra。
  if (typeof r.cron_job_id === 'string' && r.cron_job_id.length > 0) {
    extra = { ...(extra ?? {}), cron_job_id: r.cron_job_id };
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
