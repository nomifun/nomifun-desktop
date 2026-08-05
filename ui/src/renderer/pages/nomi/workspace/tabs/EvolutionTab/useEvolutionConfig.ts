/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * MIGRATION SEAM — the 进化 tab's config adapter.
 *
 * The learn / evolve settings this tab edits are TODAY stored in the
 * cross-companion shared config (`GET|PATCH /api/companion/config`, i.e.
 * `ipcBridge.companion.getSharedConfig` / `patchSharedConfig`), so every value
 * here is install-wide: changing it changes it for all companions. A follow-up
 * backend change moves these fields onto the per-companion profile.
 *
 * The `collect.*` fields of that same shared config are deliberately NOT read or
 * written here: collection is machine-level and owned solely by
 * `pages/settings/privacy` (设置 › 数据采集). This tab only links there.
 *
 * This module is the ONLY place in the tab that knows the rest is shared. It
 * already exposes a per-companion-shaped API (`useEvolutionConfig(companionId)` →
 * `{ learn, evolve, patchLearn, patchEvolve, loading }`),
 * so the migration is a rewrite of this file alone:
 *   - swap the two ipcBridge calls for `getCompanion` / `patchCompanion`,
 *   - key the fetch on `companionId` (already accepted),
 *   - flip `installWide` to false and `ownsLearningOutput` to true — the UI reads
 *     those flags to decide whether to print the "applies to all companions" /
 *     "what the loop produces lands on the default companion" notes, so both
 *     honest disclosures disappear by themselves once the values really are
 *     per-companion.
 * No section component needs to change.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import type {
  ICompanionEvolveConfig,
  ICompanionModelRef,
  ICompanionSharedConfig,
} from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';

/** Periodic-learning settings (定时学习). */
export interface EvolutionLearnConfig {
  enabled: boolean;
  interval_minutes: number;
  model: ICompanionModelRef | null;
}

/** Skill-generation settings (技能生成). Thresholds stay internal to the tab. */
export type EvolutionEvolveConfig = ICompanionEvolveConfig;

export interface EvolutionConfigHandle {
  learn: EvolutionLearnConfig | null;
  evolve: EvolutionEvolveConfig | null;
  loading: boolean;
  /** Set when the config could not be read; the tab shows a retry instead of empty sections. */
  error: string | null;
  /** True while these values are stored install-wide (pre-migration). */
  installWide: boolean;
  /**
   * True when what the background loop produces would actually land on THIS
   * companion. The backend files every mined skill AND every distilled memory
   * under one resolved owner (the default companion, else the oldest), so on any
   * other companion both sections must say so rather than imply the skills and
   * memories show up on its own 技能 / 记忆 tabs.
   */
  ownsLearningOutput: boolean;
  retry: () => void;
  patchLearn: (patch: Partial<EvolutionLearnConfig>) => Promise<void>;
  patchEvolve: (patch: Partial<EvolutionEvolveConfig>) => Promise<void>;
}

/**
 * Live view of one companion's learning/evolution settings, with optimistic
 * writes so switches don't lag the round-trip. A failed write rolls back to the
 * server's truth and rethrows, so callers can surface the error.
 */
export const useEvolutionConfig = (companionId: CompanionId | null): EvolutionConfigHandle => {
  const [config, setConfig] = useState<ICompanionSharedConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const aliveRef = useRef(true);

  // A read failure must not become an unhandled rejection: it is surfaced as a
  // retryable error state, because silently leaving `learn`/`evolve` null renders
  // a tab with two sections missing and no explanation.
  const refresh = useCallback(async () => {
    try {
      const next = await ipcBridge.companion.getSharedConfig.invoke();
      if (aliveRef.current) {
        setConfig(next);
        setError(null);
      }
    } catch (e) {
      if (aliveRef.current) setError(String(e));
    } finally {
      if (aliveRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    aliveRef.current = true;
    void refresh();
    const unsub = ipcBridge.companion.onConfigUpdated.on((evt) => {
      if (evt.scope === 'shared') void refresh();
    });
    return () => {
      aliveRef.current = false;
      unsub();
    };
  }, [refresh]);

  const retry = useCallback(() => {
    setLoading(true);
    void refresh();
  }, [refresh]);

  const patch = useCallback(
    async (
      apply: (prev: ICompanionSharedConfig) => ICompanionSharedConfig,
      request: Parameters<typeof ipcBridge.companion.patchSharedConfig.invoke>[0]
    ) => {
      setConfig((prev) => (prev ? apply(prev) : prev));
      try {
        const saved = await ipcBridge.companion.patchSharedConfig.invoke(request);
        if (aliveRef.current) setConfig(saved);
      } catch (e) {
        void refresh();
        throw e;
      }
    },
    [refresh]
  );

  const patchLearn = useCallback(
    (next: Partial<EvolutionLearnConfig>) =>
      patch((prev) => ({ ...prev, learn: { ...prev.learn, ...next } }), { learn: next }),
    [patch]
  );

  const patchEvolve = useCallback(
    (next: Partial<EvolutionEvolveConfig>) =>
      patch((prev) => ({ ...prev, evolve: { ...prev.evolve, ...next } }), { evolve: next }),
    [patch]
  );

  return {
    learn: config?.learn ?? null,
    evolve: config?.evolve ?? null,
    loading,
    error,
    installWide: true,
    // Only claim "not this companion" when the pointer positively says so: the
    // first companion ever created becomes the default, so a null pointer means
    // "nothing to warn about" rather than "some other companion owns them".
    ownsLearningOutput:
      config?.default_companion_id == null || companionId == null || config.default_companion_id === companionId,
    retry,
    patchLearn,
    patchEvolve,
  };
};
