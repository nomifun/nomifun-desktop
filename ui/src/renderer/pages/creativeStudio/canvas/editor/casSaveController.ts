/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeProjectDocument, CreativeProjectSummary } from '../../domain';
import { isCreativeProjectRepositoryError } from '../../services';

export const CREATIVE_CANVAS_SAVE_DEBOUNCE_MS = 600;

export type CanvasCasSaveStatus =
  | 'idle'
  | 'dirty'
  | 'saving'
  | 'saved'
  | 'conflict'
  | 'error';

export interface CanvasCasSaveSnapshot {
  status: CanvasCasSaveStatus;
  revision: string | null;
  hasPendingChanges: boolean;
  error: Error | null;
}

export type CanvasCasFlushResult =
  | { status: 'noop'; revision: string | null }
  | { status: 'saved'; revision: string }
  | { status: 'conflict'; revision: string; error: Error }
  | { status: 'error'; revision: string; error: Error };

export type CanvasCasSaveOperation = (
  expectedRevision: string,
  document: CreativeProjectDocument
) => Promise<Pick<CreativeProjectSummary, 'revision'>>;

export interface CanvasSaveScheduler {
  setTimeout(callback: () => void, delayMs: number): ReturnType<typeof setTimeout>;
  clearTimeout(timer: ReturnType<typeof setTimeout>): void;
}

export function canvasSaveRequiresUnloadGuard(
  snapshot: CanvasCasSaveSnapshot
): boolean {
  return snapshot.revision !== null && snapshot.hasPendingChanges;
}

const defaultScheduler: CanvasSaveScheduler = {
  setTimeout: (callback, delayMs) => setTimeout(callback, delayMs),
  clearTimeout: (timer) => clearTimeout(timer),
};

const documentSignature = (document: CreativeProjectDocument): string => JSON.stringify(document);
const cloneDocument = (document: CreativeProjectDocument): CreativeProjectDocument => structuredClone(document);

/**
 * Debounced compare-and-swap save coordinator.
 *
 * A revision conflict permanently blocks automatic saves until the owner
 * explicitly calls `reset` with a freshly loaded remote document. No force
 * write or silent retry path exists.
 */
export class CanvasCasSaveController {
  private readonly listeners = new Set<() => void>();
  private readonly saveOperation: CanvasCasSaveOperation;
  private readonly debounceMs: number;
  private readonly scheduler: CanvasSaveScheduler;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pendingDocument: CreativeProjectDocument | null = null;
  private savedSignature: string | null = null;
  private inFlight: Promise<CanvasCasFlushResult> | null = null;
  private epoch = 0;
  private snapshot: CanvasCasSaveSnapshot = {
    status: 'idle',
    revision: null,
    hasPendingChanges: false,
    error: null,
  };

  constructor(
    saveOperation: CanvasCasSaveOperation,
    options: {
      debounceMs?: number;
      scheduler?: CanvasSaveScheduler;
    } = {}
  ) {
    this.saveOperation = saveOperation;
    this.debounceMs = Math.max(0, options.debounceMs ?? CREATIVE_CANVAS_SAVE_DEBOUNCE_MS);
    this.scheduler = options.scheduler ?? defaultScheduler;
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  readonly getSnapshot = (): CanvasCasSaveSnapshot => this.snapshot;

  reset(revision: string, document: CreativeProjectDocument): void {
    this.epoch += 1;
    this.cancelTimer();
    this.inFlight = null;
    this.pendingDocument = cloneDocument(document);
    this.savedSignature = documentSignature(document);
    this.updateSnapshot({
      status: 'idle',
      revision,
      hasPendingChanges: false,
      error: null,
    });
  }

  queue(document: CreativeProjectDocument): void {
    if (this.snapshot.revision === null) return;
    this.pendingDocument = cloneDocument(document);
    const hasPendingChanges = documentSignature(document) !== this.savedSignature;

    if (this.snapshot.status === 'conflict') {
      this.updateSnapshot({ ...this.snapshot, hasPendingChanges: true });
      return;
    }

    if (!hasPendingChanges) {
      this.cancelTimer();
      this.updateSnapshot({
        ...this.snapshot,
        status: 'saved',
        hasPendingChanges: false,
        error: null,
      });
      return;
    }

    this.updateSnapshot({
      ...this.snapshot,
      status: this.snapshot.status === 'saving' ? 'saving' : 'dirty',
      hasPendingChanges: true,
      error: null,
    });
    if (this.snapshot.status !== 'saving') this.schedule();
  }

  async flush(): Promise<CanvasCasFlushResult> {
    this.cancelTimer();
    const revision = this.snapshot.revision;
    if (
      this.snapshot.status === 'conflict' &&
      this.snapshot.error &&
      revision !== null
    ) {
      return {
        status: 'conflict',
        revision,
        error: this.snapshot.error,
      };
    }
    if (revision === null || !this.pendingDocument || !this.snapshot.hasPendingChanges) {
      return { status: 'noop', revision };
    }
    if (this.inFlight) {
      await this.inFlight;
      return this.flush();
    }

    const epoch = this.epoch;
    const document = cloneDocument(this.pendingDocument);
    const signature = documentSignature(document);
    this.updateSnapshot({
      ...this.snapshot,
      status: 'saving',
      error: null,
    });

    const operation = this.performSave(epoch, revision, document, signature);
    this.inFlight = operation;
    const result = await operation;
    if (this.inFlight === operation) this.inFlight = null;

    if (
      result.status === 'saved' &&
      this.snapshot.status === 'dirty' &&
      this.snapshot.hasPendingChanges
    ) {
      return this.flush();
    }
    return result;
  }

  dispose(): void {
    this.epoch += 1;
    this.cancelTimer();
    this.listeners.clear();
  }

  private async performSave(
    epoch: number,
    expectedRevision: string,
    document: CreativeProjectDocument,
    signature: string
  ): Promise<CanvasCasFlushResult> {
    try {
      const project = await this.saveOperation(expectedRevision, document);
      if (epoch !== this.epoch) return { status: 'noop', revision: this.snapshot.revision };

      this.savedSignature = signature;
      const pendingSignature = this.pendingDocument
        ? documentSignature(this.pendingDocument)
        : signature;
      const hasPendingChanges = pendingSignature !== signature;
      this.updateSnapshot({
        status: hasPendingChanges ? 'dirty' : 'saved',
        revision: project.revision,
        hasPendingChanges,
        error: null,
      });
      return { status: 'saved', revision: project.revision };
    } catch (cause) {
      if (epoch !== this.epoch) return { status: 'noop', revision: this.snapshot.revision };
      const error = cause instanceof Error ? cause : new Error(String(cause));
      const conflict =
        isCreativeProjectRepositoryError(cause) && cause.kind === 'revision-conflict';
      this.updateSnapshot({
        ...this.snapshot,
        status: conflict ? 'conflict' : 'error',
        hasPendingChanges: true,
        error,
      });
      return conflict
        ? { status: 'conflict', revision: expectedRevision, error }
        : { status: 'error', revision: expectedRevision, error };
    }
  }

  private schedule(): void {
    this.cancelTimer();
    this.timer = this.scheduler.setTimeout(() => {
      this.timer = null;
      void this.flush();
    }, this.debounceMs);
  }

  private cancelTimer(): void {
    if (this.timer === null) return;
    this.scheduler.clearTimeout(this.timer);
    this.timer = null;
  }

  private updateSnapshot(snapshot: CanvasCasSaveSnapshot): void {
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener();
  }
}
