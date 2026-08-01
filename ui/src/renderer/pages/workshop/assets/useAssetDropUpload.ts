/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Shared file-picker / drag-and-drop upload wiring + infinite-scroll handler
 * for the asset library surfaces (standalone page and canvas drawer). Wraps a
 * `useAssetLibrary` controller; the caller renders the hidden `<input>` and
 * attaches the returned handlers to its own containers.
 */

import { useCallback, useRef, useState } from 'react';

import type { useAssetLibrary } from './useAssetLibrary';

export function useAssetDropUpload(
  lib: ReturnType<typeof useAssetLibrary>,
  options: {
    /** Distance (px) from the bottom of the scroll container that triggers `loadMore`. */
    scrollThreshold: number;
  }
) {
  const { scrollThreshold } = options;

  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [dragActive, setDragActive] = useState(false);
  const dragDepth = useRef(0);

  // ─── Upload wiring ──────────────────────────────────────────────────────────
  const openFilePicker = useCallback(() => fileInputRef.current?.click(), []);

  const onFileInputChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const files = Array.from(e.target.files ?? []);
      if (files.length) lib.startUploads(files);
      e.target.value = '';
    },
    [lib]
  );

  const isFileDrag = (e: React.DragEvent) => Array.from(e.dataTransfer.types).includes('Files');

  const onDragEnter = useCallback((e: React.DragEvent) => {
    if (!isFileDrag(e)) return;
    dragDepth.current += 1;
    setDragActive(true);
  }, []);

  const onDragOver = useCallback((e: React.DragEvent) => {
    if (!isFileDrag(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
  }, []);

  const onDragLeave = useCallback((e: React.DragEvent) => {
    if (!isFileDrag(e)) return;
    dragDepth.current -= 1;
    if (dragDepth.current <= 0) {
      dragDepth.current = 0;
      setDragActive(false);
    }
  }, []);

  const onDrop = useCallback(
    (e: React.DragEvent) => {
      if (!isFileDrag(e)) return;
      e.preventDefault();
      dragDepth.current = 0;
      setDragActive(false);
      const files = Array.from(e.dataTransfer.files);
      if (files.length) lib.startUploads(files);
    },
    [lib]
  );

  // ─── Infinite scroll ────────────────────────────────────────────────────────
  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el || lib.loadingMore || !lib.hasMore) return;
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - scrollThreshold) lib.loadMore();
  }, [lib, scrollThreshold]);

  return {
    fileInputRef,
    scrollRef,
    dragActive,
    openFilePicker,
    onFileInputChange,
    onDragEnter,
    onDragOver,
    onDragLeave,
    onDrop,
    onScroll,
  };
}
