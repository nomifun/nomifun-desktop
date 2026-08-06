/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * The 进化 tab's config adapter — one companion's 学习 / 技能进化 settings.
 *
 * These settings used to live on the cross-companion shared config
 * (`GET|PATCH /api/companion/config`), so every value here was install-wide and
 * each section had to carry an honest "applies to every companion" note. They are
 * per companion since 2026-08: this hook reads and writes THIS companion's
 * profile (`getCompanion` / `patchCompanion`), the loop runs from this
 * companion's own event cursor, and what it produces — memories, mined skills,
 * XP, mood — belongs to this companion. Both disclosures are therefore gone,
 * along with the two flags that gated them.
 *
 * The `collect.*` fields of the shared config are deliberately NOT read or
 * written here: collection is machine-level (which events this DEVICE records)
 * and owned solely by `pages/settings/privacy` (设置 › 数据采集). This tab only
 * links there.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import type {
  ICompanionEvolveConfig,
  ICompanionLearnConfig,
  ICompanionProfile,
} from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';

/** Periodic-learning settings (定时学习). */
export type EvolutionLearnConfig = ICompanionLearnConfig;

/** Skill-generation settings (技能生成). Thresholds stay internal to the tab. */
export type EvolutionEvolveConfig = ICompanionEvolveConfig;

export interface EvolutionConfigHandle {
  learn: EvolutionLearnConfig | null;
  evolve: EvolutionEvolveConfig | null;
  loading: boolean;
  /** Set when the config could not be read; the tab shows a retry instead of empty sections. */
  error: string | null;
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
  const [profile, setProfile] = useState<ICompanionProfile | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const aliveRef = useRef(true);

  // A read failure must not become an unhandled rejection: it is surfaced as a
  // retryable error state, because silently leaving `learn`/`evolve` null renders
  // a tab with two sections missing and no explanation.
  const refresh = useCallback(async () => {
    if (!companionId) {
      if (aliveRef.current) {
        setProfile(null);
        setLoading(false);
      }
      return;
    }
    try {
      const next = await ipcBridge.companion.getCompanion.invoke({ companion_id: companionId });
      if (aliveRef.current) {
        setProfile(next);
        setError(null);
      }
    } catch (e) {
      if (aliveRef.current) setError(String(e));
    } finally {
      if (aliveRef.current) setLoading(false);
    }
  }, [companionId]);

  useEffect(() => {
    aliveRef.current = true;
    setLoading(true);
    void refresh();
    // A profile-scoped `companion.config-updated` carries `scope === companion_id`;
    // only this companion's own writes (from anywhere — the sidebar, MCP, another
    // window) may refresh this pane.
    const unsub = ipcBridge.companion.onConfigUpdated.on((evt) => {
      if (companionId && evt.scope === companionId) void refresh();
    });
    return () => {
      aliveRef.current = false;
      unsub();
    };
  }, [companionId, refresh]);

  const retry = useCallback(() => {
    setLoading(true);
    void refresh();
  }, [refresh]);

  const patch = useCallback(
    async (
      apply: (prev: ICompanionProfile) => ICompanionProfile,
      request: Parameters<typeof ipcBridge.companion.patchCompanion.invoke>[0]['patch']
    ) => {
      if (!companionId) return;
      setProfile((prev) => (prev ? apply(prev) : prev));
      try {
        const saved = await ipcBridge.companion.patchCompanion.invoke({
          companion_id: companionId,
          patch: request,
        });
        if (aliveRef.current) setProfile(saved);
      } catch (e) {
        void refresh();
        throw e;
      }
    },
    [companionId, refresh]
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
    learn: profile?.learn ?? null,
    evolve: profile?.evolve ?? null,
    loading,
    error,
    retry,
    patchLearn,
    patchEvolve,
  };
};
