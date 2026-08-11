/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Serializes preference mutations while preserving "latest user choice wins"
 * UI semantics. Operations are never sent concurrently, and callbacks from an
 * older failed operation cannot roll back a newer optimistic choice.
 */
export class SerializedLatestWriteQueue {
  private tail: Promise<void> = Promise.resolve();
  private generation = 0;
  private pending = 0;

  get hasPending(): boolean {
    return this.pending > 0;
  }

  isLatest(generation: number): boolean {
    return generation === this.generation;
  }

  enqueue(
    operation: () => Promise<unknown>,
    callbacks: {
      onLatestError?: (error: unknown, generation: number) => void | Promise<void>;
      onLatestSettled?: (generation: number) => void | Promise<void>;
    } = {}
  ): { generation: number; done: Promise<void> } {
    const generation = ++this.generation;
    this.pending += 1;

    const run = this.tail.then(operation);
    const done = run
      .then(() => undefined)
      .catch(async (error: unknown) => {
        if (this.isLatest(generation)) {
          await callbacks.onLatestError?.(error, generation);
        }
      })
      .finally(async () => {
        this.pending -= 1;
        if (this.isLatest(generation)) {
          await callbacks.onLatestSettled?.(generation);
        }
      });

    // `done` handles the operation error, so the next mutation always runs.
    this.tail = done;
    return { generation, done };
  }
}
