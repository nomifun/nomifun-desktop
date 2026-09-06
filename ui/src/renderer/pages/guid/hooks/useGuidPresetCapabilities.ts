/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { agentPlatform } from '@/common/adapter/ipcBridge';
import type {
  AgentPresetDocument,
} from '@/common/types/agentPlatform';
import type { AgentPresetId } from '@/common/types/ids';
import { requiredResourceKindsForCapabilityReferences } from '@/renderer/hooks/agent/useAgentCapabilityResources';
import { useEffect, useState } from 'react';

export type GuidPresetCapabilityState = {
  capabilityIds: ReadonlySet<string>;
  requiredResourceKinds: ReadonlySet<string>;
  isLoading: boolean;
  error: Error | undefined;
};

const emptyState = (): GuidPresetCapabilityState => ({
  capabilityIds: new Set<string>(),
  requiredResourceKinds: new Set<string>(),
  isLoading: false,
  error: undefined,
});

/**
 * Resolve target-resource requirements from the exact stable Revision
 * references and the current capability catalog. Resource instances are never
 * read or persisted here; only the catalog-declared kinds are projected.
 */
export const requiredResourceKindsForDocument = (
  document: AgentPresetDocument,
  catalog: Parameters<typeof requiredResourceKindsForCapabilityReferences>[1]
): Set<string> =>
  requiredResourceKindsForCapabilityReferences(
    [...document.initial_capabilities, ...document.on_demand_capabilities].map(
      (selection) => selection.capability
    ),
    catalog
  );

export const useGuidPresetCapabilities = (
  presetId: AgentPresetId | undefined
): GuidPresetCapabilityState => {
  const [state, setState] = useState<GuidPresetCapabilityState>(emptyState);

  useEffect(() => {
    let cancelled = false;
    if (!presetId) {
      setState(emptyState());
      return undefined;
    }

    setState({
      capabilityIds: new Set<string>(),
      requiredResourceKinds: new Set<string>(),
      isLoading: true,
      error: undefined,
    });
    void Promise.all([
      agentPlatform.getEditor.invoke({ preset_id: presetId }),
      agentPlatform.capabilities.invoke(),
    ])
      .then(([editor, catalog]) => {
        if (cancelled) return;
        // Guid launches only executable summaries, so prefer the persisted
        // stable Revision over a transient editor draft whenever available.
        const document = editor.revision?.document ?? editor.draft.document;
        const capabilityIds = new Set(
          [
            ...document.initial_capabilities,
            ...document.on_demand_capabilities,
          ].map((selection) => selection.capability.id)
        );
        setState({
          capabilityIds,
          requiredResourceKinds: requiredResourceKindsForDocument(document, catalog),
          isLoading: false,
          error: undefined,
        });
      })
      .catch((error) => {
        if (cancelled) return;
        const normalizedError =
          error instanceof Error ? error : new Error('Failed to resolve Agent capabilities');
        console.error('Failed to load selected Agent capabilities:', normalizedError);
        setState({
          capabilityIds: new Set<string>(),
          requiredResourceKinds: new Set<string>(),
          isLoading: false,
          error: normalizedError,
        });
      });

    return () => {
      cancelled = true;
    };
  }, [presetId]);

  return state;
};
