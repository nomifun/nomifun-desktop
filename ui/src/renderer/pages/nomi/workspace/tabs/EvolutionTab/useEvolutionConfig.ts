/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * MIGRATION SEAM — the 进化 tab's config adapter.
 *
 * The learn / evolve / learning-source settings this tab edits are TODAY stored
 * in the cross-companion shared config (`GET|PATCH /api/companion/config`, i.e.
 * `ipcBridge.companion.getSharedConfig` / `patchSharedConfig`), so every value
 * here is install-wide: changing it changes it for all companions. A follow-up
 * backend change moves these fields onto the per-companion profile.
 *
 * This module is the ONLY place in the tab that knows that. It already exposes a
 * per-companion-shaped API (`useEvolutionConfig(companionId)` →
 * `{ learn, evolve, sources, patchLearn, patchEvolve, patchSources, loading }`),
 * so the migration is a rewrite of this file alone:
 *   - swap the two ipcBridge calls for `getCompanion` / `patchCompanion`,
 *   - key the fetch on `companionId` (already accepted),
 *   - flip `installWide` to false and `ownsGeneratedSkills` to true — the UI reads
 *     those flags to decide whether to print the "applies to all companions" /
 *     "mined skills land on the default companion" notes, so both honest
 *     disclosures disappear by themselves once the values really are
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

/**
 * The recorded event sources that may feed learning. Deliberately only these
 * three: terminal metadata, the retention/capacity policy and the raw event
 * counters are machine-level privacy controls that live in app settings, not in
 * a companion's own page.
 */
export const LEARNING_SOURCE_KEYS = ['tool_calls', 'chat_user_messages', 'requirements'] as const;

export type LearningSourceKey = (typeof LEARNING_SOURCE_KEYS)[number];

export type LearningSources = Record<LearningSourceKey, boolean>;

export interface EvolutionConfigHandle {
  learn: EvolutionLearnConfig | null;
  evolve: EvolutionEvolveConfig | null;
  sources: LearningSources | null;
  loading: boolean;
  /** Set when the config could not be read; the tab shows a retry instead of empty sections. */
  error: string | null;
  /** True while these values are stored install-wide (pre-migration). */
  installWide: boolean;
  /**
   * True when auto-mined skills would actually land on THIS companion. The
   * backend files every mined skill under the default companion
   * (`registry.resolve_default`), so on any other companion the 技能生成 section
   * must say so rather than imply the skills appear on its own 技能 tab.
   */
  ownsGeneratedSkills: boolean;
  retry: () => void;
  patchLearn: (patch: Partial<EvolutionLearnConfig>) => Promise<void>;
  patchEvolve: (patch: Partial<EvolutionEvolveConfig>) => Promise<void>;
  patchSources: (patch: Partial<LearningSources>) => Promise<void>;
}

const pickSources = (config: ICompanionSharedConfig): LearningSources => ({
  tool_calls: config.collect.tool_calls,
  chat_user_messages: config.collect.chat_user_messages,
  requirements: config.collect.requirements,
});

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

  const patchSources = useCallback(
    (next: Partial<LearningSources>) =>
      patch((prev) => ({ ...prev, collect: { ...prev.collect, ...next } }), { collect: next }),
    [patch]
  );

  return {
    learn: config?.learn ?? null,
    evolve: config?.evolve ?? null,
    sources: config ? pickSources(config) : null,
    loading,
    error,
    installWide: true,
    // Only claim "not this companion" when the pointer positively says so: the
    // first companion ever created becomes the default, so a null pointer means
    // "nothing to warn about" rather than "some other companion owns them".
    ownsGeneratedSkills:
      config?.default_companion_id == null || companionId == null || config.default_companion_id === companionId,
    retry,
    patchLearn,
    patchEvolve,
    patchSources,
  };
};
