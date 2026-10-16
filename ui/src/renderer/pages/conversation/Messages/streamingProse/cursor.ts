/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export type ProseFrameClock = {
  now: () => number;
  request: (callback: FrameRequestCallback) => number;
  cancel: (id: number) => void;
};

type Cursor = {
  /** An immutable reference to the authoritative source, never an independently appended body. */
  source: string;
  /** The immutable source reference admitted on the last presentation frame. */
  frameSource: string;
  boundaries: number[];
  visible: number;
  credit: number;
  lastFrame: number;
  deadline: number;
  revision: number;
};

const MAX_RETAINED_CURSORS = 128;
const MAX_LIVE_SOURCES = 256;
const CATCH_UP_MS = 600;
const GRAPHEMES_PER_SECOND = 48;
const segmenter = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

function boundariesFor(source: string, previous?: Cursor): number[] {
  // Revisit the previous final cluster: a new delta can extend it with a
  // combining mark, variation selector, flag or a joined emoji sequence.
  const start = previous && source.startsWith(previous.source)
    ? previous.boundaries[Math.max(0, previous.boundaries.length - 2)]
    : 0;
  const boundaries = previous ? previous.boundaries.filter(offset => offset < start) : [];
  boundaries.push(start);
  for (const part of segmenter.segment(source.slice(start))) {
    boundaries.push(start + part.index + part.segment.length);
  }
  return boundaries;
}

function boundaryIndex(boundaries: number[], offset: number): number {
  let low = 0;
  let high = boundaries.length - 1;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (boundaries[middle] < offset) low = middle + 1;
    else high = middle;
  }
  return low;
}

/** Renderer-only display progress. It does not change Session facts or busy state. */
export class ProseCursorStore {
  private readonly cursors = new Map<string, Cursor>();
  private readonly pendingSources = new Set<string>();
  private readonly listeners = new Map<string, Set<() => void>>();
  private readonly liveSources = new Map<string, { first: number; last: number }>();
  private committedSources: ReadonlyMap<string, string> = new Map();
  private snapshots = new WeakMap<object, number>();
  private ingressGeneration = 0;
  private committedGeneration = -1;
  private frame: number | undefined;
  private immediate = false;

  constructor(private readonly clock: ProseFrameClock) {}

  markLive = (identity: string): void => {
    const previous = this.liveSources.get(identity);
    const generation = ++this.ingressGeneration;
    this.liveSources.delete(identity);
    this.liveSources.set(identity, { first: previous?.first ?? generation, last: generation });
    if (this.liveSources.size > MAX_LIVE_SOURCES) {
      this.liveSources.delete(this.liveSources.keys().next().value!);
    }
  };

  isLive = (identity: string, ...through: [] | [number | undefined]): boolean => {
    const generation = through.length ? through[0] : this.ingressGeneration;
    const pending = this.liveSources.get(identity);
    return generation !== undefined && pending !== undefined && pending.first <= generation;
  };
  generation = (): number => this.ingressGeneration;
  noteSnapshot = (snapshot: object, generation: number): void => { this.snapshots.set(snapshot, generation); };
  generationFor = (snapshot: object): number | undefined => this.snapshots.get(snapshot);

  /** Commit raw renderer sources even when their process section is collapsed.
   * Child prose effects run first, so an already-admitted cursor is preserved.
   * The watermark consumes only ingress observed by this committed render. */
  commitSources = (sources: ReadonlyMap<string, string>, throughGeneration: number | undefined): void => {
    if (throughGeneration !== undefined && throughGeneration < this.committedGeneration) return;
    for (const [identity, source] of sources) {
      const previous = this.committedSources.get(identity);
      const pending = this.liveSources.get(identity);
      const changed = previous !== source;
      const admitted = throughGeneration !== undefined && pending !== undefined && pending.first <= throughGeneration;
      if (changed && admitted) {
        // A hidden history prefix was already readable before this live delta.
        // Seed only that source when it first needs a presentation cursor.
        if (!this.cursors.has(identity) && previous !== undefined) this.update(identity, previous, false);
        this.update(identity, source, true);
      } else {
        const cursor = this.cursors.get(identity);
        if (cursor && cursor.source !== source) this.update(identity, source, false);
      }
      // Separate hook queues share the list provider. A user echo can commit
      // while another queue still holds a prose delta; unchanged raw text is
      // not evidence that this source's ingress was part of that snapshot.
      if (admitted && changed) {
        if (pending.last <= throughGeneration) this.liveSources.delete(identity);
        else pending.first = throughGeneration + 1;
      }
    }
    for (const [identity] of this.cursors) {
      if (sources.has(identity)) continue;
      this.cursors.delete(identity);
      this.pendingSources.delete(identity);
      this.notify(identity);
    }
    // Keep the caller's immutable raw-source map, not another copied body map.
    this.committedSources = sources;
    if (throughGeneration !== undefined) this.committedGeneration = throughGeneration;
    this.cancelIdleFrame();
  };

  revoke = (identity: string): void => {
    this.liveSources.delete(identity);
    const cursor = this.cursors.get(identity);
    if (cursor) this.finish(identity, cursor);
    this.cancelIdleFrame();
  };

  subscribe = (identity: string, listener: () => void): (() => void) => {
    let listeners = this.listeners.get(identity);
    if (!listeners) {
      listeners = new Set();
      this.listeners.set(identity, listeners);
    }
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
      if (!listeners.size) this.listeners.delete(identity);
      this.prune();
    };
  };

  revision = (identity: string): number => this.cursors.get(identity)?.revision ?? 0;

  visible(identity: string, source: string, animate: boolean, forceImmediate = false): number {
    if (forceImmediate || this.immediate) return source.length;
    const cursor = this.cursors.get(identity);
    if (!cursor) return animate && this.committedSources.get(identity) !== source ? 0 : source.length;
    // Corrections, replacement and shortening become visible synchronously,
    // without ever showing stale text from the previous source revision.
    if (!source.startsWith(cursor.source)) return source.length;
    if (source !== cursor.source && !animate) return source.length;
    return Math.min(cursor.visible, source.length);
  }

  frameSource(identity: string, source: string, animate: boolean, forceImmediate = false): string {
    if (forceImmediate || this.immediate) return source;
    const cursor = this.cursors.get(identity);
    if (!cursor) return animate && this.committedSources.get(identity) !== source ? '' : source;
    if (!source.startsWith(cursor.source) || (source !== cursor.source && !animate)) return source;
    return cursor.frameSource;
  }

  update(identity: string, source: string, animate: boolean, forceImmediate = false): void {
    const previous = this.cursors.get(identity);
    if (previous?.source === source) {
      if (forceImmediate || this.immediate) this.finish(identity, previous);
      this.cancelIdleFrame();
      return;
    }

    const now = this.clock.now();
    const append = previous !== undefined && source.startsWith(previous.source);
    const immediate = forceImmediate || !animate || this.immediate || (previous !== undefined && !append)
      || (!previous && this.committedSources.get(identity) === source);
    // An already-visible history row needs only a baseline, not a grapheme
    // index. If it later receives live text, index its source on that append.
    const boundaries = immediate
      ? source.length ? [0, source.length] : [0]
      : boundariesFor(source, append ? previous : undefined);
    const visible = immediate ? source.length : previous?.visible ?? 0;
    const cursor: Cursor = {
      source,
      frameSource: immediate ? source : previous?.frameSource ?? '',
      boundaries,
      visible: boundaries[boundaryIndex(boundaries, visible)],
      credit: append ? previous.credit : 0,
      lastFrame: previous && previous.visible < previous.source.length ? previous.lastFrame : now,
      // A late large delta gets its own presentation budget. Reusing the old
      // burst deadline would reveal it all on the very next frame. The rate
      // remains proportional to backlog, so sustained input still catches up.
      deadline: now + CATCH_UP_MS,
      revision: (previous?.revision ?? 0) + 1,
    };
    this.cursors.delete(identity);
    this.cursors.set(identity, cursor);
    if (cursor.visible < source.length || cursor.frameSource !== source) this.pendingSources.add(identity);
    else this.pendingSources.delete(identity);
    this.notify(identity);
    this.prune();
    this.cancelIdleFrame();
    this.schedule();
  }

  setImmediate(immediate: boolean): void {
    this.immediate = immediate;
    if (immediate) {
      for (const [identity, cursor] of this.cursors) this.finish(identity, cursor);
      this.cancelIdleFrame();
    }
  }

  dispose(): void {
    if (this.frame !== undefined) this.clock.cancel(this.frame);
    this.frame = undefined;
    this.cursors.clear();
    this.pendingSources.clear();
    this.liveSources.clear();
    this.committedSources = new Map();
    this.snapshots = new WeakMap();
    this.ingressGeneration = 0;
    this.committedGeneration = -1;
    this.listeners.clear();
  }

  private notify(identity: string): void {
    for (const listener of this.listeners.get(identity) ?? []) listener();
  }

  private finish(identity: string, cursor: Cursor): void {
    this.pendingSources.delete(identity);
    if (cursor.visible === cursor.source.length && cursor.frameSource === cursor.source) return;
    cursor.visible = cursor.source.length;
    cursor.frameSource = cursor.source;
    cursor.credit = 0;
    cursor.revision++;
    this.notify(identity);
    this.prune();
  }

  private cancelIdleFrame(): void {
    if (this.frame !== undefined && !this.pendingSources.size) {
      this.clock.cancel(this.frame);
      this.frame = undefined;
    }
  }

  private schedule(): void {
    if (this.frame === undefined && this.pendingSources.size) this.frame = this.clock.request(this.advance);
  }

  private advance = (now: number): void => {
    this.frame = undefined;
    for (const identity of this.pendingSources) {
      const cursor = this.cursors.get(identity)!;
      if (now >= cursor.deadline || this.immediate || cursor.visible === cursor.source.length) {
        this.finish(identity, cursor);
        continue;
      }
      const sourceChanged = cursor.frameSource !== cursor.source;
      cursor.frameSource = cursor.source;
      const index = boundaryIndex(cursor.boundaries, cursor.visible);
      const remaining = cursor.boundaries.length - 1 - index;
      const rate = Math.max(GRAPHEMES_PER_SECOND / 1000, remaining / Math.max(1, cursor.deadline - cursor.lastFrame));
      cursor.credit += rate * Math.max(0, now - cursor.lastFrame);
      cursor.lastFrame = now;
      const step = Math.min(remaining, Math.floor(cursor.credit));
      if (!step && !sourceChanged) continue;
      cursor.credit -= step;
      cursor.visible = cursor.boundaries[index + step];
      if (cursor.visible === cursor.source.length) this.pendingSources.delete(identity);
      cursor.revision++;
      this.notify(identity);
    }
    this.schedule();
  };

  private prune(): void {
    if (this.cursors.size <= MAX_RETAINED_CURSORS) return;
    for (const [identity] of this.cursors) {
      // A transient process→public remount must not evict a pending cursor,
      // even when a long transcript has many currently mounted history rows.
      if (this.listeners.has(identity) || this.pendingSources.has(identity)) continue;
      this.cursors.delete(identity);
      this.pendingSources.delete(identity);
      if (this.cursors.size <= MAX_RETAINED_CURSORS) break;
    }
    this.cancelIdleFrame();
  }
}
