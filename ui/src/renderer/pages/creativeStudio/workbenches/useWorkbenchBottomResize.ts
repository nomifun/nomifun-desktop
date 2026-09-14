/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent, PointerEvent as ReactPointerEvent } from 'react';

const MAX_HEIGHT = 300;
const RESULTS_RESERVE = 160;

function readHeight(key: string): number | null {
  try {
    const value = Number(localStorage.getItem(key));
    // Older saved sizes are clamped to the current bounds when displayed.
    return Number.isFinite(value) && value > 0 ? value : null;
  } catch {
    return null;
  }
}

/** The default is intrinsic content height; window constraints never overwrite preferences. */
export function useWorkbenchBottomResize(storageKey: string, active: boolean) {
  const panelRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const cleanupDragRef = useRef<(() => void) | null>(null);
  const [preferredHeight, setPreferredHeight] = useState(() => readHeight(storageKey));
  const [dragHeight, setDragHeight] = useState<number | null>(null);
  const [bounds, setBounds] = useState({ min: 0, max: MAX_HEIGHT });
  const boundsRef = useRef(bounds);
  boundsRef.current = bounds;

  useLayoutEffect(() => {
    if (!active) return;
    const panel = panelRef.current;
    const content = contentRef.current;
    const workspace = panel?.parentElement;
    if (!panel || !content || !workspace) return;
    const measure = () => {
      const available = workspace.clientHeight;
      const max = available > 0
        ? Math.floor(Math.min(MAX_HEIGHT, available - Math.min(RESULTS_RESERVE, available * 0.35)))
        : MAX_HEIGHT;
      const min = Math.min(Math.ceil(content.getBoundingClientRect().height) + 1, max);
      boundsRef.current = { min, max };
      setBounds((previous) => previous.min === min && previous.max === max ? previous : { min, max });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(content);
    observer.observe(workspace);
    return () => {
      observer.disconnect();
      cleanupDragRef.current?.();
    };
  }, [active]);

  const clamp = useCallback((height: number) => Math.max(boundsRef.current.min, Math.min(boundsRef.current.max, height)), []);
  const save = useCallback((height: number | null) => {
    setPreferredHeight(height);
    setDragHeight(null);
    try {
      if (height === null) localStorage.removeItem(storageKey);
      else localStorage.setItem(storageKey, String(height));
    } catch {
      // Preferences are optional when browser storage is unavailable.
    }
  }, [storageKey]);

  const onPointerDown = useCallback((event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.pointerType !== 'touch' && event.button !== 0) return;
    event.preventDefault();
    cleanupDragRef.current?.();
    const startY = event.clientY;
    const startHeight = panelRef.current?.getBoundingClientRect().height ?? boundsRef.current.min;
    const pointerId = event.pointerId;
    const handle = event.currentTarget;
    const originalCursor = document.body.style.cursor;
    const originalUserSelect = document.body.style.userSelect;
    document.body.style.cursor = 'row-resize';
    document.body.style.userSelect = 'none';
    let latest = startHeight;
    let moved = false;
    const cleanup = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', finish);
      window.removeEventListener('pointercancel', cancel);
      window.removeEventListener('blur', cancel);
      handle.removeEventListener('lostpointercapture', cancel);
      if (handle.hasPointerCapture?.(pointerId)) handle.releasePointerCapture(pointerId);
      document.body.style.cursor = originalCursor;
      document.body.style.userSelect = originalUserSelect;
      cleanupDragRef.current = null;
    };
    const cancel = () => {
      cleanup();
      setDragHeight(null);
    };
    const finish = (pointer: PointerEvent) => {
      if (pointer.pointerId !== pointerId) return;
      latest = clamp(startHeight + startY - pointer.clientY);
      cleanup();
      if (moved || pointer.clientY !== startY) save(latest);
    };
    const move = (pointer: PointerEvent) => {
      if (pointer.pointerId !== pointerId) return;
      if (pointer.buttons === 0) { finish(pointer); return; }
      moved = true;
      latest = clamp(startHeight + startY - pointer.clientY);
      setDragHeight(latest);
    };
    cleanupDragRef.current = cleanup;
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', cancel);
    window.addEventListener('blur', cancel);
    handle.addEventListener('lostpointercapture', cancel);
    try { handle.setPointerCapture?.(pointerId); } catch { /* Window listeners also cover capture failures. */ }
  }, [clamp, save]);

  const onKeyDown = useCallback((event: KeyboardEvent<HTMLDivElement>) => {
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    if (event.key === 'Home') { save(null); return; }
    if (event.key === 'End') { save(boundsRef.current.max); return; }
    const current = panelRef.current?.getBoundingClientRect().height || boundsRef.current.min;
    save(clamp(current + (event.key === 'ArrowUp' ? 10 : -10)));
  }, [clamp, save]);

  const requested = dragHeight ?? preferredHeight;
  const height = requested === null ? undefined : clamp(requested);
  return {
    panelRef, contentRef, height, maxHeight: bounds.max,
    minHeight: bounds.min, currentHeight: height ?? bounds.min,
    onPointerDown, onKeyDown, reset: () => save(null),
  };
}
