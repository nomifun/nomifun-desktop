/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';

import { creativeTaskHistoryClient } from '../../tasks/historyClient';

import type { StandaloneWorkbenchHistoryScope } from './model';
import {
  appendStandaloneHistoryPage,
  loadStandaloneWorkbenchHistoryBootstrap,
  STANDALONE_HISTORY_PAGE_LIMIT,
  type StandaloneTaskHistoryReader,
} from './loader';
import type { CreativeTask } from '../../tasks';

export interface StandaloneWorkbenchHistoryState {
  status: 'loading' | 'ready' | 'error';
  tasks: readonly CreativeTask[];
  activeTasks: readonly CreativeTask[];
  nextCursor: string | null;
  error: Error | null;
  refreshing: boolean;
  loadingMore: boolean;
  reload(): Promise<void>;
  loadMore(): Promise<void>;
}

interface HistoryDataState {
  status: StandaloneWorkbenchHistoryState['status'];
  tasks: CreativeTask[];
  activeTasks: CreativeTask[];
  nextCursor: string | null;
  error: Error | null;
  refreshing: boolean;
  loadingMore: boolean;
}

const initialState = (): HistoryDataState => ({
  status: 'loading',
  tasks: [],
  activeTasks: [],
  nextCursor: null,
  error: null,
  refreshing: false,
  loadingMore: false,
});

const asError = (reason: unknown): Error =>
  reason instanceof Error ? reason : new Error(String(reason));

export function useStandaloneWorkbenchHistory(
  scope: StandaloneWorkbenchHistoryScope,
  reader: StandaloneTaskHistoryReader = creativeTaskHistoryClient
): StandaloneWorkbenchHistoryState {
  const [state, setState] = useState<HistoryDataState>(initialState);
  const stateRef = useRef(state);
  stateRef.current = state;
  const generationRef = useRef(0);
  const requestRef = useRef<AbortController | null>(null);
  const moreRequestRef = useRef<AbortController | null>(null);

  const loadInitial = useCallback(
    async (preserve: boolean): Promise<void> => {
      const generation = generationRef.current + 1;
      generationRef.current = generation;
      requestRef.current?.abort();
      moreRequestRef.current?.abort();
      const controller = new AbortController();
      requestRef.current = controller;
      setState((current) =>
        preserve && current.status === 'ready'
          ? { ...current, error: null, refreshing: true, loadingMore: false }
          : initialState()
      );
      try {
        const next = await loadStandaloneWorkbenchHistoryBootstrap(
          reader,
          scope,
          controller.signal
        );
        if (generationRef.current !== generation || controller.signal.aborted) return;
        setState({
          status: 'ready',
          ...next,
          error: null,
          refreshing: false,
          loadingMore: false,
        });
      } catch (reason) {
        if (generationRef.current !== generation || controller.signal.aborted) return;
        const error = asError(reason);
        setState((current) =>
          preserve && current.tasks.length > 0
            ? { ...current, status: 'ready', error, refreshing: false }
            : { ...initialState(), status: 'error', error }
        );
      }
    },
    [reader, scope.workbenchKind]
  );

  useEffect(() => {
    void loadInitial(false);
    return () => {
      generationRef.current += 1;
      requestRef.current?.abort();
      moreRequestRef.current?.abort();
    };
  }, [loadInitial]);

  const reload = useCallback(() => loadInitial(true), [loadInitial]);

  const loadMore = useCallback(async (): Promise<void> => {
    const current = stateRef.current;
    if (
      current.status !== 'ready' ||
      current.loadingMore ||
      !current.nextCursor
    ) {
      return;
    }
    const generation = generationRef.current;
    const requestedCursor = current.nextCursor;
    moreRequestRef.current?.abort();
    const controller = new AbortController();
    moreRequestRef.current = controller;
    setState((value) => ({ ...value, loadingMore: true, error: null }));
    try {
      const page = await reader.listStandalone(
        {
          ...scope,
          limit: STANDALONE_HISTORY_PAGE_LIMIT,
          cursor: requestedCursor,
        },
        controller.signal
      );
      if (generationRef.current !== generation || controller.signal.aborted) return;
      setState((value) => ({
        ...value,
        tasks: appendStandaloneHistoryPage(value.tasks, page),
        nextCursor: page.nextCursor,
        loadingMore: false,
      }));
    } catch (reason) {
      if (generationRef.current !== generation || controller.signal.aborted) return;
      setState((value) => ({
        ...value,
        error: asError(reason),
        loadingMore: false,
      }));
    }
  }, [reader, scope.workbenchKind]);

  return { ...state, reload, loadMore };
}
