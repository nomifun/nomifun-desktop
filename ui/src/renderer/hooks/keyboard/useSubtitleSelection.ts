import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from 'react';
import type { KeyboardSelection, KeyboardState } from '@/common/keyboard';
import { subtitleTextMap } from '@/renderer/pages/keyboardSubtitle/subtitleText';

type Bookmark = KeyboardSelection & { document: string; active: boolean };
export type SubtitleCaret = { left: number; top: number; height: number };

/** Never focuses a window/element: only preserve a user's existing editing selection. */
export function useSubtitleSelection(state: KeyboardState | null, source: RefObject<HTMLSpanElement | null>,
  viewport: RefObject<HTMLDivElement | null>, editable: boolean, composing: boolean) {
  const saved = useRef<Bookmark | null>(null);
  const previousDocument = useRef<string | null>(null);
  const appliedDictation = useRef<string | null>(null);
  const latest = useRef({ state, editable, composing }); latest.current = { state, editable, composing };
  const [caret, setCaret] = useState<SubtitleCaret | null>(null);
  const clear = useCallback(() => { saved.current = null; setCaret(null); }, []);
  const deactivate = useCallback(() => { if (saved.current) saved.current.active = false; setCaret(null); }, []);
  const ownsFocus = useCallback(() => Boolean(source.current && document.hasFocus() && document.activeElement === source.current), [source]);
  const capture = useCallback(() => {
    const root = source.current; const selection = window.getSelection(); const current = latest.current.state;
    if (!root || !ownsFocus() || !current?.visible || !current.documentId || !selection?.anchorNode || !selection.focusNode
      || !root.contains(selection.anchorNode) || !root.contains(selection.focusNode)) return;
    const map = subtitleTextMap(root);
    saved.current = { document: current.documentId, active: true,
      anchor: map.offset(selection.anchorNode, selection.anchorOffset), focus: map.offset(selection.focusNode, selection.focusOffset) };
  }, [ownsFocus, source]);
  const forCommit = useCallback((): KeyboardSelection | null => {
    capture();
    const bookmark = saved.current;
    return bookmark && bookmark.document === latest.current.state?.documentId ? { anchor: bookmark.anchor, focus: bookmark.focus } : null;
  }, [capture]);
  const preserve = useCallback(() => {
    const root = source.current; const bookmark = saved.current; const current = latest.current;
    if (!root || !bookmark?.active || !current.state?.visible || current.state.documentId !== bookmark.document || !ownsFocus()) {
      setCaret(null); return;
    }
    if (current.composing) return;
    const map = subtitleTextMap(root);
    const replacing = current.state.phase !== 'idle' && current.state.partial ? current.state.dictationSelection : null;
    const displayOffset = (offset: number) => replacing && offset > Math.min(replacing.anchor, replacing.focus)
      ? Math.max(Math.min(replacing.anchor, replacing.focus), offset - Math.abs(replacing.anchor - replacing.focus)) : offset;
    const anchor = map.locate(displayOffset(bookmark.anchor)); const focus = map.locate(displayOffset(bookmark.focus));
    const selection = window.getSelection();
    if (selection && (selection.anchorNode !== anchor[0] || selection.anchorOffset !== anchor[1]
      || selection.focusNode !== focus[0] || selection.focusOffset !== focus[1])) {
      selection.setBaseAndExtent(anchor[0], anchor[1], focus[0], focus[1]);
    }
    if (current.editable || bookmark.anchor !== bookmark.focus) { setCaret(null); return; }
    const range = document.createRange(); range.setStart(focus[0], focus[1]); range.collapse(true);
    let rect = typeof range.getClientRects === 'function' ? Array.from(range.getClientRects()).find(value => value.height > 0) : undefined;
    let x = rect?.left;
    if (!rect && focus[0].nodeType === Node.TEXT_NODE && focus[0].textContent?.length) {
      const length = focus[0].textContent.length;
      range.setStart(focus[0], Math.max(0, focus[1] - 1)); range.setEnd(focus[0], Math.min(length, focus[1] || 1));
      rect = typeof range.getClientRects === 'function' ? Array.from(range.getClientRects()).at(-1) : undefined;
      x = focus[1] ? rect?.right : rect?.left;
    }
    if (!rect && !map.text) { rect = root.getBoundingClientRect(); x = rect.left + rect.width / 2; }
    const bounds = viewport.current?.getBoundingClientRect(); const origin = root.parentElement?.getBoundingClientRect();
    if (!rect || x === undefined || !bounds || !origin || rect.height <= 0 || x < bounds.left || x > bounds.right
      || rect.top < bounds.top || rect.bottom > bounds.bottom) { setCaret(null); return; }
    const next = { left: x - origin.left, top: rect.top - origin.top, height: rect.height };
    setCaret(previous => previous?.left === next.left && previous.top === next.top && previous.height === next.height ? previous : next);
  }, [ownsFocus, source, viewport]);
  useLayoutEffect(() => {
    if (previousDocument.current !== state?.documentId || !state?.visible) clear();
    previousDocument.current = state?.documentId ?? null;
    const stamp = state?.phase === 'idle' && state.dictationSelection
      ? `${state.documentId}:${state.revision}:${state.dictationSelection.anchor}:${state.dictationSelection.focus}` : null;
    if (stamp && stamp !== appliedDictation.current && saved.current?.active && state?.dictationSelection) {
      saved.current.anchor = state.dictationSelection.anchor; saved.current.focus = state.dictationSelection.focus;
    }
    if (state?.phase === 'idle') appliedDictation.current = stamp;
    preserve();
  }, [state?.documentId, state?.visible, state?.phase, state?.text, state?.partial, state?.audioState.state,
    state?.dictationSelection?.anchor, state?.dictationSelection?.focus, state?.settings.subtitleFontSize, editable, composing, clear, preserve]);
  useEffect(() => {
    const changed = () => {
      const selection = window.getSelection(); const root = source.current;
      if (selection?.rangeCount && (!root?.contains(selection.anchorNode) || !root.contains(selection.focusNode))) { deactivate(); return; }
      if (latest.current.editable && !latest.current.composing) capture();
    };
    const resized = () => preserve(); const scroller = viewport.current;
    document.addEventListener('selectionchange', changed);
    window.addEventListener('blur', deactivate); window.addEventListener('resize', resized);
    scroller?.addEventListener('scroll', resized, { passive: true });
    return () => {
      document.removeEventListener('selectionchange', changed);
      window.removeEventListener('blur', deactivate); window.removeEventListener('resize', resized);
      scroller?.removeEventListener('scroll', resized);
    };
  }, [capture, deactivate, preserve, source, viewport]);
  return { caret, capture, clear, deactivate, preserve, forCommit };
}
