/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isCreativeProjectRepositoryError } from "../../services";
import {
  cloneDirectorState,
  exportDirectorProjectV1,
  type DirectorState,
} from "../domain";
import type { DirectorProjectBaseline } from "./directorProjectPersistence";

export const DIRECTOR_SAVE_DEBOUNCE_MS = 600;

export type DirectorCasSaveStatus =
  "idle" | "dirty" | "saving" | "saved" | "conflict" | "error";

export interface DirectorCasSaveSnapshot {
  status: DirectorCasSaveStatus;
  revision: string | null;
  hasPendingChanges: boolean;
  error: Error | null;
}

export type DirectorCasFlushResult =
  | { status: "noop"; revision: string | null }
  | { status: "saved"; revision: string }
  | { status: "conflict"; revision: string; error: Error }
  | { status: "error"; revision: string; error: Error };

export type DirectorPersistOperation = (
  baseline: DirectorProjectBaseline,
  state: DirectorState,
) => Promise<DirectorProjectBaseline>;

export interface DirectorSaveScheduler {
  setTimeout(
    callback: () => void,
    delayMs: number,
  ): ReturnType<typeof setTimeout>;
  clearTimeout(timer: ReturnType<typeof setTimeout>): void;
}

const defaultScheduler: DirectorSaveScheduler = {
  setTimeout: (callback, delayMs) => setTimeout(callback, delayMs),
  clearTimeout: (timer) => clearTimeout(timer),
};

function directorSignature(state: DirectorState): string {
  const exported = exportDirectorProjectV1(state);
  if (!exported.ok) {
    throw new TypeError(
      `Invalid Director state at ${exported.error.path}: ${exported.error.message}`,
    );
  }
  return exported.json;
}

/**
 * Debounced CAS owner for the Director state. Runtime-only playback and capture
 * operation flags are removed by the v1 serializer, so they never create fake
 * persistence churn.
 */
export class DirectorCasSaveController {
  private readonly listeners = new Set<() => void>();
  private readonly persist: DirectorPersistOperation;
  private readonly debounceMs: number;
  private readonly scheduler: DirectorSaveScheduler;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private baseline: DirectorProjectBaseline | null = null;
  private pendingState: DirectorState | null = null;
  private savedSignature: string | null = null;
  private inFlight: Promise<DirectorCasFlushResult> | null = null;
  private epoch = 0;
  private snapshot: DirectorCasSaveSnapshot = {
    status: "idle",
    revision: null,
    hasPendingChanges: false,
    error: null,
  };

  constructor(
    persist: DirectorPersistOperation,
    options: { debounceMs?: number; scheduler?: DirectorSaveScheduler } = {},
  ) {
    this.persist = persist;
    this.debounceMs = Math.max(
      0,
      options.debounceMs ?? DIRECTOR_SAVE_DEBOUNCE_MS,
    );
    this.scheduler = options.scheduler ?? defaultScheduler;
  }

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  readonly getSnapshot = (): DirectorCasSaveSnapshot => this.snapshot;

  getBaseline(): DirectorProjectBaseline | null {
    return this.baseline;
  }

  reset(baseline: DirectorProjectBaseline): void {
    this.epoch += 1;
    this.cancelTimer();
    this.inFlight = null;
    this.baseline = baseline;
    this.pendingState = cloneDirectorState(baseline.state);
    this.savedSignature = directorSignature(baseline.state);
    this.updateSnapshot({
      status: "idle",
      revision: baseline.project.revision,
      hasPendingChanges: false,
      error: null,
    });
  }

  queue(state: DirectorState): void {
    if (!this.baseline) return;
    this.pendingState = cloneDirectorState(state);
    const hasPendingChanges = directorSignature(state) !== this.savedSignature;
    if (!hasPendingChanges) {
      this.cancelTimer();
      this.updateSnapshot({
        ...this.snapshot,
        status: "saved",
        hasPendingChanges: false,
        error: null,
      });
      return;
    }
    if (this.snapshot.status === "conflict") {
      this.updateSnapshot({ ...this.snapshot, hasPendingChanges: true });
      return;
    }
    this.updateSnapshot({
      ...this.snapshot,
      status: this.snapshot.status === "saving" ? "saving" : "dirty",
      hasPendingChanges: true,
      error: null,
    });
    if (this.snapshot.status !== "saving") this.schedule();
  }

  async flush(): Promise<DirectorCasFlushResult> {
    this.cancelTimer();
    const baseline = this.baseline;
    if (!baseline || !this.pendingState || !this.snapshot.hasPendingChanges) {
      return { status: "noop", revision: this.snapshot.revision };
    }
    if (this.snapshot.status === "conflict" && this.snapshot.error) {
      return {
        status: "conflict",
        revision: baseline.project.revision,
        error: this.snapshot.error,
      };
    }
    if (this.inFlight) {
      await this.inFlight;
      return this.flush();
    }

    const epoch = this.epoch;
    const state = cloneDirectorState(this.pendingState);
    const signature = directorSignature(state);
    this.updateSnapshot({ ...this.snapshot, status: "saving", error: null });
    const operation = this.performSave(epoch, baseline, state, signature);
    this.inFlight = operation;
    const result = await operation;
    if (this.inFlight === operation) this.inFlight = null;
    if (
      result.status === "saved" &&
      this.snapshot.status === "dirty" &&
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
    baseline: DirectorProjectBaseline,
    state: DirectorState,
    signature: string,
  ): Promise<DirectorCasFlushResult> {
    try {
      const nextBaseline = await this.persist(baseline, state);
      if (epoch !== this.epoch)
        return { status: "noop", revision: this.snapshot.revision };
      this.baseline = nextBaseline;
      this.savedSignature = signature;
      const pendingSignature = this.pendingState
        ? directorSignature(this.pendingState)
        : signature;
      const hasPendingChanges = pendingSignature !== signature;
      this.updateSnapshot({
        status: hasPendingChanges ? "dirty" : "saved",
        revision: nextBaseline.project.revision,
        hasPendingChanges,
        error: null,
      });
      return { status: "saved", revision: nextBaseline.project.revision };
    } catch (cause) {
      if (epoch !== this.epoch)
        return { status: "noop", revision: this.snapshot.revision };
      const error = cause instanceof Error ? cause : new Error(String(cause));
      const conflict =
        isCreativeProjectRepositoryError(cause) &&
        cause.kind === "revision-conflict";
      this.updateSnapshot({
        ...this.snapshot,
        status: conflict ? "conflict" : "error",
        hasPendingChanges: true,
        error,
      });
      return conflict
        ? { status: "conflict", revision: baseline.project.revision, error }
        : { status: "error", revision: baseline.project.revision, error };
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

  private updateSnapshot(snapshot: DirectorCasSaveSnapshot): void {
    this.snapshot = snapshot;
    for (const listener of this.listeners) listener();
  }
}
