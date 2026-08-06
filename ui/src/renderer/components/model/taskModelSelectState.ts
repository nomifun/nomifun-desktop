/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * The decision logic behind {@link TaskModelSelect}, extracted as a pure
 * function.
 *
 * Eight surfaces in this renderer had each grown their own copy of "group by
 * provider, render a stale reference as a disabled (unavailable) option, and
 * explain an empty catalog" — and they disagreed, which is how a perfectly
 * valid model ended up flagged as retired on one page and not another. This is
 * the one answer; the component only renders it.
 */

import type { IProvider } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import type { TaskModelGroup } from '@/renderer/hooks/agent/useModelsForTask';

/** A stored `(provider, model)` reference, plus the voice id for TTS slots. */
export interface TaskModelSelection {
  provider_id: ProviderId;
  model: string;
  voice?: string | null;
}

/**
 * Which providers the first select offers.
 *
 * - `'task'`: only providers that actually own a model for this task. Right for
 *   secondary slots (learning, ASR, TTS…) where an empty provider is noise.
 * - `'all-enabled'`: every enabled provider. Right for the companion's main
 *   chat model, where hiding a provider would render the SAVED provider as a
 *   raw uuid and make the user's own configuration look corrupt.
 */
export type TaskModelProviderScope = 'task' | 'all-enabled';

export interface TaskModelSelectState {
  /** Providers for the first select, in selector order. */
  providers: IProvider[];
  /** Task-capable models of the drafted provider, in catalog order. */
  models: string[];
  /** The saved provider is not in `providers` (deleted or disabled). */
  providerStale: boolean;
  /** The saved provider is fine but the saved model is no longer offered. */
  modelStale: boolean;
  /** The catalog holds at least one model for this task, anywhere. */
  anyModel: boolean;
  /** The saved reference resolves to a live provider AND a live model. */
  configured: boolean;
}

export const taskModelSelectState = ({
  groups,
  enabledProviders,
  scope,
  value,
  draftProviderId,
  isLoading,
}: {
  groups: readonly TaskModelGroup[];
  enabledProviders: readonly IProvider[];
  scope: TaskModelProviderScope;
  value: TaskModelSelection | null;
  draftProviderId: ProviderId | null;
  isLoading: boolean;
}): TaskModelSelectState => {
  const providers = scope === 'task' ? groups.map((g) => g.provider) : [...enabledProviders];
  const currentProvider = providers.find((p) => p.id === draftProviderId);
  const models = currentProvider
    ? (groups.find((g) => g.provider.id === currentProvider.id)?.models ?? [])
    : [];

  // The saved model belongs to the DRAFTED provider only; after a provider
  // switch the model select starts empty instead of showing the old pick.
  const savedModel = value != null && value.provider_id === draftProviderId ? value.model : null;
  const modelValid = savedModel != null && models.includes(savedModel);

  // While the catalog is unresolved (loading, or a failed resolve that
  // useModelsForTask deliberately reports as still-loading) nothing may be
  // called stale: a saved reference is unknown, not wrong.
  const providerStale = !isLoading && draftProviderId != null && currentProvider === undefined;
  // Reported only when the provider itself is live: a saved model under a
  // deleted provider is ONE problem, and telling the user about both would ask
  // them to re-pick a model in a select that has nothing to offer.
  const modelStale = !isLoading && !providerStale && savedModel != null && !modelValid;

  return {
    providers,
    models,
    providerStale,
    modelStale,
    anyModel: !isLoading && groups.length > 0,
    configured: !isLoading && !providerStale && modelValid,
  };
};
