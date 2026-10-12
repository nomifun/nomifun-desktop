/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { agentPlatform } from '@/common/adapter/ipcBridge';
import type {
  AgentPresetDocument,
  AgentCatalogResponse,
} from '@/common/types/agentPlatform';
import type { AgentPresetId } from '@/common/types/ids';
import type { IIdmmConfig } from '@/common/types/idmm';
import { createDefaultIdmmConfig } from '@/common/types/idmm';
import { requiredResourceKindsForCapabilitySelections } from '@/renderer/hooks/agent/useAgentCapabilityResources';
import { useEffect, useState } from 'react';

export type GuidPresetCapabilityState = {
  capabilityIds: ReadonlySet<string>;
  actionIds: ReadonlySet<string>;
  requiredResourceKinds: ReadonlySet<string>;
  idmm: IIdmmConfig;
  isLoading: boolean;
  error: Error | undefined;
};

const emptyState = (): GuidPresetCapabilityState => ({
  capabilityIds: new Set<string>(),
  actionIds: new Set<string>(),
  requiredResourceKinds: new Set<string>(),
  idmm: createDefaultIdmmConfig(),
  isLoading: false,
  error: undefined,
});

/**
 * Resolve target-resource requirements from the exact stable Revision
 * action grants and the current Module catalog. Resource instances are never
 * read or persisted here; only the declared Action resource kinds are projected.
 */
export const requiredResourceKindsForDocument = (
  document: AgentPresetDocument,
  catalog: Pick<AgentCatalogResponse, 'modules'>
): Set<string> =>
  requiredResourceKindsForCapabilitySelections(
    document.enabled_capabilities,
    catalog.modules
  );

/** Load the saved capability and resource defaults used by Guid. */
export const useGuidPresetCapabilities = (
  presetId: AgentPresetId | undefined,
  revisionDigest?: string,
): GuidPresetCapabilityState => {
  const sourceKey = presetId ? `${presetId}:${revisionDigest ?? 'draft'}` : undefined;
  const [state, setState] = useState<{
    sourceKey: string | undefined;
    value: GuidPresetCapabilityState;
  }>(() => ({ sourceKey: undefined, value: emptyState() }));

  useEffect(() => {
    let cancelled = false;
    if (!presetId) {
      setState({ sourceKey, value: emptyState() });
      return undefined;
    }

    setState({ sourceKey, value: { ...emptyState(), isLoading: true } });
    void Promise.all([
      agentPlatform.getEditor.invoke({ preset_id: presetId }),
      agentPlatform.catalog.invoke(),
    ])
      .then(([editor, catalog]) => {
        if (cancelled) return;
        // Saved official and personal Agents use the stable Revision. A newly
        // initialized official configuration has only its baseline draft until
        // its first launch or save; reading it must never prepare a Revision.
        const document = editor.revision?.document ?? editor.draft.document;
        const capabilityIds = new Set(
          document.enabled_capabilities.map((selection) => selection.capability.id)
        );
        const actionIds = new Set(
          document.enabled_capabilities.flatMap(
            (selection) => selection.action_allowlist ?? []
          )
        );
        setState({
          sourceKey,
          value: {
            capabilityIds,
            actionIds,
            requiredResourceKinds: requiredResourceKindsForDocument(document, catalog),
            idmm: structuredClone(
              document.runtime_policy?.idmm ?? createDefaultIdmmConfig()
            ),
            isLoading: false,
            error: undefined,
          },
        });
      })
      .catch((error) => {
        if (cancelled) return;
        const normalizedError =
          error instanceof Error ? error : new Error('Failed to resolve Agent capabilities');
        console.error('Failed to load selected Agent capabilities:', normalizedError);
        setState({ sourceKey, value: { ...emptyState(), error: normalizedError } });
      });

    return () => {
      cancelled = true;
    };
  }, [presetId, revisionDigest, sourceKey]);

  // Effect cleanup and the next request happen after render. Fence the old
  // requirements immediately when the same Agent gains a new saved Revision.
  return state.sourceKey === sourceKey
    ? state.value
    : { ...emptyState(), isLoading: Boolean(presetId) };
};
