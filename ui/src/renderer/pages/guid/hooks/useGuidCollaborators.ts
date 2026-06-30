/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import type { TModelRef } from '@/common/types/orchestrator/orchestratorTypes';
import { useCallback, useEffect, useState } from 'react';

/**
 * Persisted「协作模型」preference for the homepage 智能编排 entry.
 *
 * The 主模型 is the existing single-model selection (`nomi.defaultModel`, owned by
 * {@link useGuidModelSelection}); this hook owns the ADDITIONAL collaborator pool
 * the planner may assign per node by difficulty. Stored in the backend client
 * settings bag under `nomi.orchestrationCollaborators` (same mechanism as the
 * default model), so the choice survives across sessions. Empty ⇒ the run uses
 * just the 主模型.
 */
export interface GuidCollaboratorsResult {
  /** The chosen collaborator (provider, model) pairs. */
  collaborators: TModelRef[];
  /** Replace + persist the collaborator selection. Never throws. */
  setCollaborators: (next: TModelRef[]) => Promise<void>;
}

export const useGuidCollaborators = (): GuidCollaboratorsResult => {
  const [collaborators, _set] = useState<TModelRef[]>([]);

  // Hydrate from the persisted preference once. Fail-soft: a missing / malformed
  // value just leaves the selection empty (= 全程用主模型).
  useEffect(() => {
    const saved = configService.get('nomi.orchestrationCollaborators');
    if (Array.isArray(saved)) {
      _set(saved.filter((r) => r && typeof r.provider_id === 'string' && typeof r.model === 'string'));
    }
  }, []);

  const setCollaborators = useCallback(async (next: TModelRef[]) => {
    _set(next);
    await configService.set('nomi.orchestrationCollaborators', next).catch((error) => {
      console.error('Failed to save orchestration collaborators:', error);
    });
  }, []);

  return { collaborators, setCollaborators };
};
