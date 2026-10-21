/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentPresetSummary, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { createDefaultIdmmConfig } from '@/common/types/idmm';
import { useAgentPresetCapabilities } from './useAgentPresetCapabilities';
import { useMemo } from 'react';

/** Template publication and saved Preset authoring are distinct launch sources. */
export function useAgentLaunchCapabilities(preset?: AgentPresetSummary, template?: OfficialPresetTemplate) {
  const saved = useAgentPresetCapabilities(
    template ? undefined : preset?.preset_id,
    template ? undefined : preset?.current_stable_revision?.revision_digest,
  );
  const published = useMemo(() => template ? {
    capabilityIds: new Set(template.seed.enabled_capabilities.map(selection => selection.capability.id)),
    actionIds: new Set(template.seed.enabled_capabilities.flatMap(selection => selection.action_allowlist ?? [])),
    requiredResourceKinds: new Set(template.seed.required_resource_kinds),
    idmm: createDefaultIdmmConfig(),
    defaultModel: null,
    isLoading: false,
    error: undefined,
  } : undefined, [template]);
  return published ? { ...published, refresh: saved.refresh } : saved;
}
