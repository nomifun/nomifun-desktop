/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useRef, useState } from 'react';

import { normalizePromptLibrary } from './library';
import type { PromptLibraryItem, PromptLibraryPort } from './types';

export interface UsePromptLibraryOptions {
  enabled?: boolean;
}

export interface UsePromptLibraryResult {
  items: PromptLibraryItem[];
  loading: boolean;
  refreshing: boolean;
  error: Error | null;
  invalidCount: number;
  reload(): Promise<void>;
}

function asError(value: unknown): Error {
  return value instanceof Error ? value : new Error(String(value));
}

export function usePromptLibrary(
  port: PromptLibraryPort,
  options: UsePromptLibraryOptions = {}
): UsePromptLibraryResult {
  const enabled = options.enabled ?? true;
  const [items, setItems] = useState<PromptLibraryItem[]>([]);
  const [loading, setLoading] = useState(enabled);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [invalidCount, setInvalidCount] = useState(0);
  const itemsRef = useRef<PromptLibraryItem[]>([]);
  const requestRef = useRef(0);
  const abortRef = useRef<AbortController | null>(null);

  const reload = useCallback(async () => {
    if (!enabled) return;
    const request = ++requestRef.current;
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    setError(null);
    setLoading(itemsRef.current.length === 0);
    setRefreshing(itemsRef.current.length > 0);
    try {
      const normalized = normalizePromptLibrary(await port.list(controller.signal));
      if (controller.signal.aborted || request !== requestRef.current) return;
      itemsRef.current = normalized.items;
      setItems(normalized.items);
      setInvalidCount(normalized.invalidCount);
    } catch (reason) {
      if (controller.signal.aborted || request !== requestRef.current) return;
      setError(asError(reason));
    } finally {
      if (!controller.signal.aborted && request === requestRef.current) {
        setLoading(false);
        setRefreshing(false);
      }
    }
  }, [enabled, port]);

  useEffect(() => {
    if (!enabled) {
      abortRef.current?.abort();
      setLoading(false);
      setRefreshing(false);
      return;
    }
    void reload();
    return () => abortRef.current?.abort();
  }, [enabled, reload]);

  return { items, loading, refreshing, error, invalidCount, reload };
}
