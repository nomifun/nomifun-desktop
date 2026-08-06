/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * The one owner of `CollectConfig` in the UI.
 *
 * Collection is machine-level, not companion-level: every field here lives in the
 * cross-companion shared config (`GET|PATCH /api/companion/config`), so exactly
 * one surface — 设置 › 数据采集 — may write it. A companion page that also wrote
 * these fields would let two screens disagree about one global value, which is
 * why 进化 only links here now.
 *
 * Alongside the config this exposes the two read-only measurements the page needs
 * to be honest about what is on disk: per-source counters (`eventStats`) and the
 * event spool's size/date bounds (`eventStorage`).
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import type {
  ICompanionCollectConfig,
  ICompanionEventStorageStatus,
  ICompanionSharedConfig,
  ICompanionSourceStats,
} from '@/common/adapter/ipcBridge';

/** Collection sources in display order, most-often-asked-about first. */
export const COLLECTION_SOURCE_KEYS = [
  'tool_calls',
  'chat_user_messages',
  'requirements',
  'terminal_sessions',
  'companion_dialogues',
] as const;

export type CollectionSourceKey = (typeof COLLECTION_SOURCE_KEYS)[number];

/** How much of the user's own words a source can contain. */
export const SOURCE_SENSITIVITY: Record<CollectionSourceKey, 'low' | 'medium' | 'high'> = {
  tool_calls: 'medium',
  chat_user_messages: 'high',
  requirements: 'medium',
  terminal_sessions: 'medium',
  companion_dialogues: 'high',
};

export const RETENTION_DAYS_MIN = 7;
export const RETENTION_DAYS_MAX = 365;
export const CAPACITY_MB_MIN = 16;
export const CAPACITY_MB_MAX = 512;

/** Counters move as events stream in; refresh lightly while the page is open. */
const STATS_POLL_MS = 15_000;

export type StorageState = 'loading' | 'ready' | 'error';

export interface CollectSettingsHandle {
  collect: ICompanionCollectConfig | null;
  loading: boolean;
  /** Set when the config could not be read; the page shows a retry instead of empty sections. */
  error: string | null;
  stats: ICompanionSourceStats[];
  storage: ICompanionEventStorageStatus | null;
  storageState: StorageState;
  retry: () => void;
  patch: (patch: Partial<ICompanionCollectConfig>) => Promise<void>;
  /** Re-read both measurements (after a policy change deleted files, say). */
  refreshMeasurements: (showStorageLoading?: boolean) => void;
  disableAll: () => Promise<void>;
}

export const useCollectSettings = (): CollectSettingsHandle => {
  const [config, setConfig] = useState<ICompanionSharedConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<ICompanionSourceStats[]>([]);
  const [storage, setStorage] = useState<ICompanionEventStorageStatus | null>(null);
  const [storageState, setStorageState] = useState<StorageState>('loading');
  const aliveRef = useRef(true);
  const storageRequestRef = useRef(0);

  // A read failure must not become an unhandled rejection: it becomes a retryable
  // error state, because a silent null renders a page whose privacy controls have
  // vanished with no explanation — the exact failure this page exists to prevent.
  const refreshConfig = useCallback(async () => {
    try {
      const next = await ipcBridge.companion.getSharedConfig.invoke();
      if (!aliveRef.current) return;
      setConfig(next);
      setError(null);
    } catch (e) {
      if (aliveRef.current) setError(String(e));
    } finally {
      if (aliveRef.current) setLoading(false);
    }
  }, []);

  const refreshMeasurements = useCallback((showStorageLoading = false) => {
    void ipcBridge.companion.eventStats
      .invoke()
      .then((next) => {
        if (aliveRef.current) setStats(next);
      })
      .catch(() => {});

    // Storage reads are racy across a policy change (the PATCH prunes, so an
    // in-flight read can resolve with pre-prune numbers); only the newest wins.
    const request = ++storageRequestRef.current;
    if (showStorageLoading) {
      setStorage(null);
      setStorageState('loading');
    }
    void ipcBridge.companion.eventStorage
      .invoke()
      .then((next) => {
        if (!aliveRef.current || request !== storageRequestRef.current) return;
        setStorage(next);
        setStorageState('ready');
      })
      .catch(() => {
        if (!aliveRef.current || request !== storageRequestRef.current) return;
        setStorage(null);
        setStorageState('error');
      });
  }, []);

  useEffect(() => {
    aliveRef.current = true;
    void refreshConfig();
    refreshMeasurements(true);
    const unsubConfig = ipcBridge.companion.onConfigUpdated.on((evt) => {
      if (evt.scope === 'shared') void refreshConfig();
    });
    // Learning consumes events, which can make day-files deletable.
    const unsubLearn = ipcBridge.companion.onLearnFinished.on(() => refreshMeasurements());
    const timer = setInterval(() => {
      if (document.visibilityState === 'visible') refreshMeasurements();
    }, STATS_POLL_MS);
    return () => {
      aliveRef.current = false;
      storageRequestRef.current += 1;
      clearInterval(timer);
      unsubConfig();
      unsubLearn();
    };
  }, [refreshConfig, refreshMeasurements]);

  const retry = useCallback(() => {
    setLoading(true);
    void refreshConfig();
    refreshMeasurements(true);
  }, [refreshConfig, refreshMeasurements]);

  // Optimistic so switches don't lag the round-trip; a failed write rolls back to
  // the server's truth and rethrows so the caller can surface the error.
  const patch = useCallback(
    async (next: Partial<ICompanionCollectConfig>) => {
      setConfig((prev) => (prev ? { ...prev, collect: { ...prev.collect, ...next } } : prev));
      try {
        const saved = await ipcBridge.companion.patchSharedConfig.invoke({ collect: next });
        if (aliveRef.current) setConfig(saved);
      } catch (e) {
        void refreshConfig();
        throw e;
      }
    },
    [refreshConfig]
  );

  const disableAll = useCallback(async () => {
    const saved = await ipcBridge.companion.disableAll.invoke();
    if (aliveRef.current) setConfig(saved);
    refreshMeasurements();
  }, [refreshMeasurements]);

  return {
    collect: config?.collect ?? null,
    loading,
    error,
    stats,
    storage,
    storageState,
    retry,
    patch,
    refreshMeasurements,
    disableAll,
  };
};

/** KiB below one MiB, else MiB — the spool is capped at 512 MiB. */
export const formatBytes = (bytes: number): string => {
  if (bytes < 1024 * 1024) return `${Math.max(0, bytes / 1024).toFixed(1)} KiB`;
  return `${Math.max(0, bytes / (1024 * 1024)).toFixed(1)} MiB`;
};
