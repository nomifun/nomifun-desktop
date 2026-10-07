/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * useWorkspaceTags — like `useRequirementTags` but preserves the FULL
 * `ITagSummary` shape (per-status counts, paused flag, …) instead of narrowing
 * to `{tag, done, total}`. The workspace `RequirementFilters` types its
 * `tagOptions` as `ITagSummary[]`, so it needs the unmapped summaries.
 *
 * Subscribes to the same five live events as `useRequirementTags` and refetches
 * on any of them or a reconnect, so the tag-filter options stay in sync.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import { isHandledAuthExpiredHttpError } from '@/common/adapter/httpBridge';
import type { ITagSummary } from '@/common/adapter/ipcBridge';

export function useWorkspaceTags() {
  const [tags, setTags] = useState<ITagSummary[]>([]);
  const requestIdRef = useRef(0);
  const activeRef = useRef(false);

  const refresh = useCallback(async () => {
    if (!activeRef.current) return;
    const requestId = ++requestIdRef.current;
    try {
      const res = await ipcBridge.requirements.tags.invoke();
      if (requestId !== requestIdRef.current) return;
      setTags(res);
    } catch (e) {
      if (requestId !== requestIdRef.current) return;
      if (isHandledAuthExpiredHttpError(e)) return;
      console.error('Failed to load tags', e);
    }
  }, []);

  useEffect(() => {
    activeRef.current = true;
    void refresh();
    const unsubs = [
      ipcBridge.requirements.onCreated.on(() => void refresh()),
      ipcBridge.requirements.onUpdated.on(() => void refresh()),
      ipcBridge.requirements.onStatusChanged.on(() => void refresh()),
      ipcBridge.requirements.onDeleted.on(() => void refresh()),
      ipcBridge.requirements.onTagPaused.on(() => void refresh()),
      ipcBridge.conversation.reconnected.on(() => void refresh()),
    ];
    return () => {
      activeRef.current = false;
      ++requestIdRef.current;
      unsubs.forEach((u) => u());
    };
  }, [refresh]);

  return { tags, refresh };
}
