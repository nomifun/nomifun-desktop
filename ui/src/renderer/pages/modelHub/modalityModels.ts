/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * The modality-first projection behind the hub's 对话 / 视觉 / 嵌入与检索 sections.
 *
 * The source of truth is the `provider_models` catalog ROWS
 * (`GET /api/provider-models`), not `POST /api/model-profiles/resolve`: resolve
 * answers "which models may a selector offer", so it returns enabled rows only.
 * A management view has to show a DISABLED row too — otherwise its own toggle
 * could only turn models off and then lose them. The backend data model is
 * untouched: this is a filter over `tasks` / `traits`, nothing more.
 *
 * 视觉 is deliberately a trait-filtered `chat` projection rather than its own
 * `ModelTask`, because that is what the backend vocabulary says
 * (`ModelTrait::VisionInput` modifies `ModelTask::Chat`).
 */

import type { IProvider } from '@/common/config/storage';
import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import type { ModelTrait } from '@/common/protocolBindings/ModelTrait';
import type { ProviderModelResponse } from '@/common/protocolBindings/ProviderModelResponse';
import type { ProviderId } from '@/common/types/ids';

/** The modality sections that project `provider_models` rows by task/trait. */
export type ModalityKey = 'chat' | 'vision' | 'embedding';

export interface ModalitySpec {
  /** Membership needs ANY of these tasks. */
  tasks: readonly ModelTask[];
  /** …and EVERY one of these traits. */
  traits: readonly ModelTrait[];
}

export const MODALITY_SPECS: Record<ModalityKey, ModalitySpec> = {
  chat: { tasks: ['chat'], traits: [] },
  vision: { tasks: ['chat'], traits: ['vision_input'] },
  embedding: { tasks: ['embedding', 'rerank'], traits: [] },
};

export interface ModalityModelRow {
  providerId: ProviderId;
  model: string;
  enabled: boolean;
  description: string | null;
  tasks: ModelTask[];
  traits: ModelTrait[];
}

export interface ModalityProviderGroup {
  providerId: ProviderId;
  providerName: string;
  platform: string;
  models: ModalityModelRow[];
}

export const rowMatchesModality = (row: ProviderModelResponse, spec: ModalitySpec): boolean =>
  spec.tasks.some((task) => row.tasks.includes(task)) &&
  spec.traits.every((trait) => row.traits.includes(trait));

/**
 * A row carrying no task at all. The backend seeds `tasks` when a row is
 * created, so this is legacy/hand-edited data — it belongs to no modality and
 * would otherwise be invisible everywhere in the hub.
 */
export const isUntaggedRow = (row: ProviderModelResponse): boolean => row.tasks.length === 0;

const groupRows = (
  rows: readonly ProviderModelResponse[],
  providers: readonly IProvider[],
  providerName: (provider: IProvider) => string
): ModalityProviderGroup[] => {
  const byProvider = new Map<string, ModalityModelRow[]>();
  for (const row of rows) {
    const list = byProvider.get(row.provider_id) ?? [];
    list.push({
      providerId: row.provider_id as ProviderId,
      model: row.model,
      enabled: row.enabled,
      description: row.description ?? null,
      tasks: [...row.tasks],
      traits: [...row.traits],
    });
    byProvider.set(row.provider_id, list);
  }

  const groups: ModalityProviderGroup[] = [];
  // Provider order is the selector ordering authority (free-model platform
  // first); rows inside a provider follow the catalog `sort_order`, with the
  // model name as the tie-break so the list never reshuffles between renders.
  for (const provider of providers) {
    const models = byProvider.get(provider.id);
    if (!models || models.length === 0) continue;
    const orderOf = new Map(
      rows
        .filter((row) => row.provider_id === provider.id)
        .map((row) => [row.model, row.sort_order] as const)
    );
    models.sort((a, b) => {
      const delta = (orderOf.get(a.model) ?? 0) - (orderOf.get(b.model) ?? 0);
      return delta !== 0 ? delta : a.model.localeCompare(b.model);
    });
    groups.push({
      providerId: provider.id,
      providerName: providerName(provider),
      platform: provider.platform,
      models,
    });
  }
  return groups;
};

export const buildModalityGroups = (
  rows: readonly ProviderModelResponse[],
  providers: readonly IProvider[],
  spec: ModalitySpec,
  providerName: (provider: IProvider) => string = (provider) => provider.name
): ModalityProviderGroup[] =>
  groupRows(
    rows.filter((row) => rowMatchesModality(row, spec)),
    providers,
    providerName
  );

export const buildUntaggedGroups = (
  rows: readonly ProviderModelResponse[],
  providers: readonly IProvider[],
  providerName: (provider: IProvider) => string = (provider) => provider.name
): ModalityProviderGroup[] => groupRows(rows.filter(isUntaggedRow), providers, providerName);
