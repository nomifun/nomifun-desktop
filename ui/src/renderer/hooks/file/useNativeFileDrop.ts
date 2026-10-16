/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect, useRef, type RefObject } from 'react';
import type { DragDropEvent } from '@tauri-apps/api/webview';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';

export interface UseNativeFileDropOptions {
  targetRef: RefObject<HTMLElement | null>;
  enabled?: boolean;
  onHoverChange: (hover: boolean) => void;
  onDrop: (paths: string[]) => void;
}

/** Match native physical pixels to the visible renderer surface, including webview zoom. */
function isOverTarget(target: HTMLElement | null, position: { x: number; y: number }): boolean {
  if (!target?.isConnected || !Number.isFinite(position.x) || !Number.isFinite(position.y)) return false;
  const scale = Number.isFinite(window.devicePixelRatio) && window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
  const x = position.x / scale;
  const y = position.y / scale;
  const bounds = target.getBoundingClientRect();
  if (bounds.width <= 0 || bounds.height <= 0 || x < bounds.left || x >= bounds.right || y < bounds.top || y >= bounds.bottom) return false;
  const style = window.getComputedStyle(target);
  if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse') return false;
  const hit = target.ownerDocument.elementFromPoint(x, y);
  return hit !== null && target.contains(hit);
}

/** Tauri owns OS file drops; keep each native listener scoped to its visible DOM target. */
export function useNativeFileDrop({ targetRef, enabled = true, onHoverChange, onDrop }: UseNativeFileDropOptions): void {
  const callbacks = useRef({ onHoverChange, onDrop });
  callbacks.current = { onHoverChange, onDrop };

  useEffect(() => {
    if (!enabled || !isTauriRuntime()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let hovered = false;
    let draggingFiles = false;
    const setHovered = (next: boolean) => {
      if (next === hovered) return;
      hovered = next;
      callbacks.current.onHoverChange(next);
    };
    const handleEvent = (payload: DragDropEvent) => {
      if (disposed) return;
      if (payload.type === 'leave') {
        draggingFiles = false;
        setHovered(false);
        return;
      }
      const overTarget = isOverTarget(targetRef.current, payload.position);
      if (payload.type === 'drop') {
        draggingFiles = false;
        setHovered(false);
        if (overTarget && payload.paths.length > 0) callbacks.current.onDrop(payload.paths);
      } else {
        if (payload.type === 'enter') draggingFiles = payload.paths.length > 0;
        setHovered(draggingFiles && overTarget);
      }
    };
    void (async () => {
      try {
        const { getCurrentWebview } = await import('@tauri-apps/api/webview');
        if (disposed) return;
        const stop = await getCurrentWebview().onDragDropEvent(event => handleEvent(event.payload));
        if (disposed) stop();
        else unlisten = stop;
      } catch (error) {
        if (!disposed) console.warn('[NativeFileDrop] Failed to attach Tauri drag-drop listener:', error);
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
      setHovered(false);
    };
  }, [enabled, targetRef]);
}
