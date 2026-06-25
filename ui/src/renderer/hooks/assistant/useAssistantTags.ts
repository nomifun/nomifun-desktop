import { ipcBridge } from '@/common';
import type { AssistantTag, AssistantTagDimension, CreateAssistantTagRequest } from '@/common/types/agent/assistantTypes';
import { useCallback, useEffect, useMemo, useState } from 'react';

/** Loads the merged tag vocabulary and exposes CRUD + per-dimension views. */
export const useAssistantTags = () => {
  const [tags, setTags] = useState<AssistantTag[]>([]);
  const [loading, setLoading] = useState(false);

  const loadTags = useCallback(async () => {
    setLoading(true);
    try {
      setTags(await ipcBridge.assistantTags.list.invoke());
    } catch (error) {
      console.error('Failed to load assistant tags:', error);
      setTags([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadTags();
  }, [loadTags]);

  const audienceTags = useMemo(
    () => tags.filter((t) => t.dimension === 'audience').sort((a, b) => a.sort_order - b.sort_order),
    [tags]
  );
  const scenarioTags = useMemo(
    () => tags.filter((t) => t.dimension === 'scenario').sort((a, b) => a.sort_order - b.sort_order),
    [tags]
  );

  /** key → AssistantTag, for resolving labels on cards. */
  const tagByKey = useMemo(() => new Map(tags.map((t) => [t.key, t])), [tags]);

  const createTag = useCallback(
    async (req: CreateAssistantTagRequest) => {
      const created = await ipcBridge.assistantTags.create.invoke(req);
      await loadTags();
      return created;
    },
    [loadTags]
  );

  const renameTag = useCallback(
    async (key: string, label: string) => {
      await ipcBridge.assistantTags.update.invoke({ key, label });
      await loadTags();
    },
    [loadTags]
  );

  const deleteTag = useCallback(
    async (key: string) => {
      await ipcBridge.assistantTags.delete.invoke({ key });
      await loadTags();
    },
    [loadTags]
  );

  return { tags, audienceTags, scenarioTags, tagByKey, loading, loadTags, createTag, renameTag, deleteTag };
};

export type TagDimension = AssistantTagDimension;
