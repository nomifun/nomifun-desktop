/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { agentPlatform } from '@/common/adapter/ipcBridge';
import type {
  AgentPresetDocument,
  AgentCatalogResponse,
  ChatRouteCandidate,
} from '@/common/types/agentPlatform';
import { AGENT_CHAT_MODEL_TASK, cloneAgentPresetDocument } from '@/common/types/agentPlatform';
import type { AgentPresetId } from '@/common/types/ids';
import type { IIdmmConfig } from '@/common/types/idmm';
import { createDefaultIdmmConfig } from '@/common/types/idmm';
import { requiredResourceKindsForCapabilitySelections } from '@/renderer/hooks/agent/useAgentCapabilityResources';
import { useCallback, useEffect, useState } from 'react';

export type GuidPresetCapabilityState = {
  capabilityIds: ReadonlySet<string>;
  actionIds: ReadonlySet<string>;
  requiredResourceKinds: ReadonlySet<string>;
  idmm: IIdmmConfig;
  defaultModel: Pick<ChatRouteCandidate, 'provider_id' | 'model'> | null;
  isLoading: boolean;
  error: Error | undefined;
};

const emptyState = (): GuidPresetCapabilityState => ({
  capabilityIds: new Set<string>(),
  actionIds: new Set<string>(),
  requiredResourceKinds: new Set<string>(),
  idmm: createDefaultIdmmConfig(),
  defaultModel: null,
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
): GuidPresetCapabilityState & { refresh: () => void } => {
  const sourceKey = presetId ? `${presetId}:${revisionDigest ?? 'draft'}` : undefined;
  const [requestVersion, setRequestVersion] = useState(0);
  const requestKey = sourceKey ? `${sourceKey}:${requestVersion}` : undefined;
  const refresh = useCallback(() => setRequestVersion(version => version + 1), []);
  const [state, setState] = useState<{
    requestKey: string | undefined;
    value: GuidPresetCapabilityState;
  }>(() => ({ requestKey: undefined, value: emptyState() }));

  useEffect(() => {
    let cancelled = false;
    if (!presetId) {
      setState({ requestKey, value: emptyState() });
      return undefined;
    }

    setState({ requestKey, value: { ...emptyState(), isLoading: true } });
    let failedRequest: 'preset editor' | 'Agent catalog' | undefined;
    void Promise.all([
      agentPlatform.getEditor.invoke({ preset_id: presetId }).catch(error => {
        failedRequest ??= 'preset editor';
        throw error;
      }),
      agentPlatform.catalog.invoke().catch(error => {
        failedRequest ??= 'Agent catalog';
        throw error;
      }),
    ])
      .then(([editor, catalog]) => {
        if (cancelled) return;
        // Saved official and personal Agents use the stable Revision. A newly
        // initialized official configuration has only its baseline draft until
        // its first launch or save; reading it must never prepare a Revision.
        const document = cloneAgentPresetDocument(editor.revision?.document ?? editor.draft.document);
        const defaultModel = document.chat_route_records[AGENT_CHAT_MODEL_TASK]?.primary;
        const capabilityIds = new Set(
          document.enabled_capabilities.map((selection) => selection.capability.id)
        );
        const actionIds = new Set(
          document.enabled_capabilities.flatMap(
            (selection) => selection.action_allowlist ?? []
          )
        );
        setState({
          requestKey,
          value: {
            capabilityIds,
            actionIds,
            requiredResourceKinds: requiredResourceKindsForDocument(document, catalog),
            idmm: structuredClone(
              document.runtime_policy?.idmm ?? createDefaultIdmmConfig()
            ),
            defaultModel: defaultModel
              ? { provider_id: defaultModel.provider_id, model: defaultModel.model }
              : null,
            isLoading: false,
            error: undefined,
          },
        });
      })
      .catch((error) => {
        if (cancelled) return;
        const normalizedError =
          error instanceof Error ? error : new Error('Failed to resolve Agent capabilities');
        console.error('Failed to load selected Agent capabilities:',
          { presetId, revisionDigest, failedRequest }, normalizedError);
        setState({ requestKey, value: { ...emptyState(), error: normalizedError } });
      });

    return () => {
      cancelled = true;
    };
  }, [presetId, revisionDigest, requestKey]);

  // Effect cleanup and the next request happen after render. Fence the old
  // requirements immediately on a new saved Revision or an explicit retry.
  return {
    ...(state.requestKey === requestKey
      ? state.value
      : { ...emptyState(), isLoading: Boolean(presetId) }),
    refresh,
  };
};
