/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useState } from 'react';
import { ipcBridge } from '@/common';
import type { IKnowledgeTag } from '@/common/adapter/ipcBridge';

/**
 * CRUD hook for knowledge-base tags (categorization / filtering).
 * Mirrors `useKnowledgeBases` pattern: fetch on mount, expose mutators + refresh.
 */
export function useKnowledgeTags() {
  const [tags, setTags] = useState<IKnowledgeTag[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const res = await ipcBridge.knowledge.listTags.invoke();
      setTags(res);
      setError(null);
    } catch (e) {
      console.error('Failed to load knowledge tags', e);
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const createTag = useCallback(
    async (label: string, color?: string) => {
      const tag = await ipcBridge.knowledge.createTag.invoke({ label, color });
      await refresh();
      return tag;
    },
    [refresh]
  );

  const updateTag = useCallback(
    async (key: string, patch: { label?: string; color?: string; sortOrder?: number }) => {
      await ipcBridge.knowledge.updateTag.invoke({ key, ...patch });
      await refresh();
    },
    [refresh]
  );

  const deleteTag = useCallback(
    async (key: string) => {
      await ipcBridge.knowledge.deleteTag.invoke({ key });
      await refresh();
    },
    [refresh]
  );

  return { tags, loading, error, createTag, updateTag, deleteTag, refresh };
}
