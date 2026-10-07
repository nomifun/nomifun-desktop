import { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { ipcBridge } from '@/common';
import { isHandledAuthExpiredHttpError } from '@/common/adapter/httpBridge';
import type { IListRequirementsParams, IRequirement } from '@/common/adapter/ipcBridge';
import {
  initialRequirementTagLoadState,
  reduceRequirementTagLoadState,
} from './requirementTagLoadState';

export function useRequirements(params: IListRequirementsParams, allPages = false) {
  const [items, setItems] = useState<IRequirement[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const paramsKey = JSON.stringify([params, allPages]);
  const requestIdRef = useRef(0);
  const activeQueryRef = useRef<string | null>(null);

  const refresh = useCallback(async () => {
    // A mutation started under old filters can retain its old refresh callback.
    if (activeQueryRef.current !== paramsKey) return;
    const requestId = ++requestIdRef.current;
    setLoading(true);
    try {
      let page = params.page ?? 1;
      const collected: IRequirement[] = [];
      let total = 0;
      for (;;) {
        const res = await ipcBridge.requirements.list.invoke(allPages ? { ...params, page } : params);
        if (requestId !== requestIdRef.current) return;
        collected.push(...res.items);
        total = res.total;
        if (!allPages || !res.has_more) break;
        if (res.items.length === 0) throw new Error('Requirements pagination made no progress');
        page += 1;
      }
      setItems(collected);
      setTotal(total);
      setError(null);
    } catch (e) {
      if (requestId !== requestIdRef.current) return;
      if (isHandledAuthExpiredHttpError(e)) return;
      console.error('Failed to load requirements', e);
      setError(String(e));
    } finally {
      if (requestId === requestIdRef.current) setLoading(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paramsKey]);

  useEffect(() => {
    activeQueryRef.current = paramsKey;
    setItems([]);
    setTotal(0);
    setError(null);
    void refresh();
    return () => {
      activeQueryRef.current = null;
      ++requestIdRef.current;
    };
  }, [paramsKey, refresh]);

  // Live updates: any create/update/status/delete invalidates the current view.
  // tagPaused also affects status/visibility (a paused tag's needs_review item
  // may have just appeared), so refresh on that too.
  // WebSocket delivery has no replay: any gap (reconnect, server lag resync)
  // may have dropped requirements events, so reload the durable snapshot.
  useEffect(() => {
    const unsubs = [
      ipcBridge.requirements.onCreated.on(() => void refresh()),
      ipcBridge.requirements.onUpdated.on(() => void refresh()),
      ipcBridge.requirements.onStatusChanged.on(() => void refresh()),
      ipcBridge.requirements.onDeleted.on(() => void refresh()),
      ipcBridge.requirements.onTagPaused.on(() => void refresh()),
      ipcBridge.conversation.reconnected.on(() => void refresh()),
    ];
    return () => unsubs.forEach((u) => u());
  }, [refresh]);

  return { items, total, loading, error, refresh };
}

export function useRequirementTags() {
  const [state, dispatch] = useReducer(reduceRequirementTagLoadState, initialRequirementTagLoadState);
  const requestIdRef = useRef(0);
  const activeRef = useRef(false);
  const refresh = useCallback(async () => {
    if (!activeRef.current) return;
    const requestId = ++requestIdRef.current;
    dispatch({ type: 'start', requestId });
    try {
      const res = await ipcBridge.requirements.tags.invoke();
      if (requestId !== requestIdRef.current) return;
      dispatch({
        type: 'success',
        requestId,
        tags: res.map((tag) => ({ tag: tag.tag, done: tag.done, total: tag.total })),
      });
    } catch (e) {
      if (requestId !== requestIdRef.current) return;
      if (!isHandledAuthExpiredHttpError(e)) {
        console.error('Failed to load tags', e);
        dispatch({ type: 'failure', requestId, error: String(e) });
      }
    } finally {
      if (requestId === requestIdRef.current) dispatch({ type: 'finish', requestId });
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
      // WebSocket delivery has no replay: reload tag counts after any gap.
      ipcBridge.conversation.reconnected.on(() => void refresh()),
    ];
    return () => {
      activeRef.current = false;
      ++requestIdRef.current;
      unsubs.forEach((u) => u());
    };
  }, [refresh]);
  return { tags: state.tags, loading: state.loading, error: state.error, refresh };
}
