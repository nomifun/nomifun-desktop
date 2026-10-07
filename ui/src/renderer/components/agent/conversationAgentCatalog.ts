/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  OFFICIAL_PRESET_KEYS,
  type AgentBindingSummary,
  type AgentPresetSummary,
  type OfficialPresetKey,
  type OfficialPresetTemplate,
} from '@/common/types/agentPlatform';

/** Product-owned Agents have their own canonical conversation ingress. */
export const COMPANION_TEMPLATE_KEY =
  'companion.default' satisfies OfficialPresetKey;

export const CUSTOMER_SERVICE_TEMPLATE_KEY =
  'customer-service.default' satisfies OfficialPresetKey;

export const isConversationAgentTemplateKey = (
  value: string,
): value is OfficialPresetKey =>
  OFFICIAL_PRESET_KEYS.includes(value as OfficialPresetKey)
  && value !== COMPANION_TEMPLATE_KEY
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
