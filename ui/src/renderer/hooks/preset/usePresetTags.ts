import { ipcBridge } from '@/common';
import type {
  PresetTag,
  PresetTagDimension,
  PresetTagReference,
  CreatePresetTagRequest,
} from '@/common/types/agent/presetTypes';
import { useCallback, useEffect, useMemo, useState } from 'react';

/** Loads the merged tag vocabulary and exposes CRUD + per-dimension views. */
export const usePresetTags = () => {
  const [tags, setTags] = useState<PresetTag[]>([]);
  const [loading, setLoading] = useState(false);

  const loadTags = useCallback(async () => {
    setLoading(true);
    try {
      setTags(await ipcBridge.presetTags.list.invoke());
    } catch (error) {
      console.error('Failed to load preset tags:', error);
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

  /** key → PresetTag, for resolving labels on cards. */
  const tagByKey = useMemo(() => new Map(tags.map((t) => [t.key, t])), [tags]);

  const createTag = useCallback(
    async (req: CreatePresetTagRequest) => {
      const created = await ipcBridge.presetTags.create.invoke(req);
      await loadTags();
      return created;
    },
    [loadTags]
  );

  const renameTag = useCallback(
    async (key: PresetTagReference, label: string) => {
      await ipcBridge.presetTags.update.invoke({ key, label });
      await loadTags();
    },
    [loadTags]
  );

  const deleteTag = useCallback(
    async (key: PresetTagReference) => {
      await ipcBridge.presetTags.delete.invoke({ key });
      await loadTags();
    },
    [loadTags]
  );

  return { tags, audienceTags, scenarioTags, tagByKey, loading, loadTags, createTag, renameTag, deleteTag };
};

export type TagDimension = PresetTagDimension;
