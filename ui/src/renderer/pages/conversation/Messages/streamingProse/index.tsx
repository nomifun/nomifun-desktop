/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { createContext, useCallback, useContext, useLayoutEffect, useMemo, useSyncExternalStore } from 'react';
import MarkdownView from '@/renderer/components/Markdown';
import { ProseCursorStore } from './cursor';

const ProseContext = createContext<ProseCursorStore | undefined>(undefined);
const noSubscription = () => () => {};
const zero = () => 0;
const inactiveIngress = { markLive: (_identity: string) => {}, isLive: (_identity: string, _throughGeneration?: number) => false,
  revoke: (_identity: string) => {}, generation: zero,
  noteSnapshot: (_snapshot: object, _generation: number) => {},
  generationFor: (_snapshot: object): number | undefined => undefined,
  commitSources: (_sources: ReadonlyMap<string, string>, _throughGeneration: number | undefined) => {} };

export function StreamingProseProvider({ scope, children }: { scope?: string; children: React.ReactNode }) {
  const store = useMemo(() => new ProseCursorStore({
    now: () => performance.now(),
    request: callback => requestAnimationFrame(callback),
    cancel: id => cancelAnimationFrame(id),
  }), [scope]);

  useLayoutEffect(() => {
    const media = window.matchMedia?.('(prefers-reduced-motion: reduce)');
    const updateEnvironment = () => store.setImmediate(Boolean(media?.matches) || document.visibilityState === 'hidden');
    updateEnvironment();
    media?.addEventListener('change', updateEnvironment);
    document.addEventListener('visibilitychange', updateEnvironment);
    return () => {
      media?.removeEventListener('change', updateEnvironment);
      document.removeEventListener('visibilitychange', updateEnvironment);
      store.dispose();
    };
  }, [store]);

  return <ProseContext.Provider value={store}>{children}</ProseContext.Provider>;
}

/** Called by the live ingress path only; loading history must not mark sources. */
export function useStreamingProseIngress() {
  const store = useContext(ProseContext);
  return useMemo(() => store ? {
    markLive: store.markLive, isLive: store.isLive, revoke: store.revoke,
    generation: store.generation, commitSources: store.commitSources,
    noteSnapshot: store.noteSnapshot, generationFor: store.generationFor,
  } : inactiveIngress, [store]);
}

type ProseInput = {
  identity: string;
  text: string;
  animate: boolean;
  /** Cancellation, withdrawal and other authoritative terminal boundaries flush immediately. */
  immediate?: boolean;
  /** Optional exact canonical source/range shared by process and public views. */
  sourceText?: string;
  sourceOffset?: number;
};

export function useStreamingProse({ identity, text, animate, immediate = false,
  sourceText = text, sourceOffset = 0 }: ProseInput): number {
  const store = useContext(ProseContext);
  const matchesSource = sourceOffset >= 0 && sourceText.slice(sourceOffset, sourceOffset + text.length) === text;
  const eligible = animate && matchesSource;
  const subscribe = useCallback((listener: () => void) => store?.subscribe(identity, listener) ?? noSubscription(),
    [identity, store]);
  const snapshot = useCallback(() => store?.revision(identity) ?? 0, [identity, store]);
  useSyncExternalStore(subscribe, snapshot, zero);
  useLayoutEffect(() => {
    // Transformed/filtered projections do not advance a shared canonical
    // cursor. They render immediately rather than guessing a text offset.
    if (matchesSource) store?.update(identity, sourceText, eligible, immediate);
  }, [eligible, identity, immediate, matchesSource, sourceText, store]);
  if (!store || !matchesSource || immediate) return text.length;
  return Math.max(0, Math.min(text.length, store.visible(identity, sourceText, eligible) - sourceOffset));
}

type MarkdownProps = Omit<React.ComponentProps<typeof MarkdownView>, 'children' | 'revealOffset'>;

export function StreamingProse({ identity, text, animate, immediate, sourceText, sourceOffset,
  ...markdownProps }: ProseInput & MarkdownProps) {
  const store = useContext(ProseContext);
  const revealOffset = useStreamingProse({ identity, text, animate, immediate, sourceText, sourceOffset });
  const source = sourceText ?? text;
  const offset = sourceOffset ?? 0;
  const exactRange = offset >= 0 && source.slice(offset, offset + text.length) === text;
  // Canonical ingestion keeps its existing timing. Only the immutable source
  // reference handed to Markdown is staged by the shared presentation frame,
  // preventing several deltas in one paint from reparsing the same tail.
  const displayedText = store && exactRange
    ? store.frameSource(identity, source, animate, immediate).slice(offset, offset + text.length)
    : text;
  return <MarkdownView {...markdownProps} revealOffset={store ? Math.min(revealOffset, displayedText.length) : undefined}>
    {displayedText}
  </MarkdownView>;
}
