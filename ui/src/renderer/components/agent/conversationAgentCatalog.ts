/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  OFFICIAL_PRESET_KEYS,
  type AgentBindingSummary,
  type AgentPresetSummary,
  type AgentPresetLibraryResponse,
  type OfficialPresetKey,
  type OfficialPresetTemplate,
} from '@/common/types/agentPlatform';

/** Product-owned Agents have their own canonical conversation ingress. */
export const COMPANION_TEMPLATE_KEY =
  'companion.default' satisfies OfficialPresetKey;

export const isCompanionAgentTemplateKey = (value: string | undefined): boolean =>
  value === COMPANION_TEMPLATE_KEY;

export const CUSTOMER_SERVICE_TEMPLATE_KEY =
  'customer-service.default' satisfies OfficialPresetKey;

export const isConversationAgentTemplateKey = (
  value: string,
): value is OfficialPresetKey =>
  OFFICIAL_PRESET_KEYS.includes(value as OfficialPresetKey)
  && !isCompanionAgentTemplateKey(value)
  && value !== CUSTOMER_SERVICE_TEMPLATE_KEY;

export const isConversationAgentTemplate = (
  template: Pick<OfficialPresetTemplate, 'template_key'>,
): boolean => isConversationAgentTemplateKey(template.template_key);

/**
 * Product bindings are the authoritative ownership signal. Avoid display-name
 * or capability heuristics: a personal Agent may legitimately discuss support,
 * while a Preset bound to a Customer Service target is product-owned.
 */
export const filterConversationAgentPresets = <Preset extends AgentPresetSummary>(
  presets: readonly Preset[],
  activeBindings: readonly AgentBindingSummary[],
): Preset[] => {
  const customerServicePresetIds = new Set(
    activeBindings
      .filter((binding) => binding.target_kind === 'customer')
      .map((binding) => binding.preset_revision_ref.preset_id),
  );
  return presets.filter((preset) => !customerServicePresetIds.has(preset.preset_id));
};

/** Saved authoring choices are explicit Presets, including official customizations. */
export const savedAgentPresets = (library: AgentPresetLibraryResponse): AgentPresetSummary[] => [
  ...library.user_presets,
  ...Object.values(library.official_presets ?? {}).filter(preset => Boolean(preset.current_stable_revision)),
];

/** Product-owned official configurations remain in their dedicated selectors. */
export const conversationAgentPresets = (library: AgentPresetLibraryResponse): AgentPresetSummary[] => {
  const productPresetIds = new Set([
    library.official_presets?.[COMPANION_TEMPLATE_KEY]?.preset_id,
    library.official_presets?.[CUSTOMER_SERVICE_TEMPLATE_KEY]?.preset_id,
  ].filter(Boolean));
  return filterConversationAgentPresets(savedAgentPresets(library), library.active_bindings)
    .filter(preset => !productPresetIds.has(preset.preset_id));
};
