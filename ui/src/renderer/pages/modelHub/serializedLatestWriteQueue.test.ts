/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { SerializedLatestWriteQueue } from './serializedLatestWriteQueue';

const deferred = <T = void>() => {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
};

const flushMicrotasks = async () => {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
};

describe('SerializedLatestWriteQueue', () => {
  test('sends rapid preference writes to the server in user-operation order', async () => {
    const queue = new SerializedLatestWriteQueue();
    const first = deferred();
    const second = deferred();
    const started: string[] = [];

    const firstWrite = queue.enqueue(async () => {
      started.push('A');
      await first.promise;
    });
    const secondWrite = queue.enqueue(async () => {
      started.push('B');
      await second.promise;
    });

    await flushMicrotasks();
    expect(started).toEqual(['A']);
    expect(queue.hasPending).toBe(true);

    first.resolve(undefined);
    await firstWrite.done;
    await flushMicrotasks();
    expect(started).toEqual(['A', 'B']);

    second.resolve(undefined);
    await secondWrite.done;
    expect(queue.hasPending).toBe(false);
  });

  test('does not let an older late failure roll back the latest choice', async () => {
    const queue = new SerializedLatestWriteQueue();
    const first = deferred();
    const second = deferred();
    const rolledBack: string[] = [];

    const firstWrite = queue.enqueue(
      () => first.promise,
      {
        onLatestError: () => {
          rolledBack.push('A');
        },
      }
    );
    const secondWrite = queue.enqueue(
      () => second.promise,
      {
        onLatestError: () => {
          rolledBack.push('B');
        },
      }
    );

    first.reject(new Error('late A failure'));
    await firstWrite.done;
    expect(rolledBack).toEqual([]);

    second.reject(new Error('B failure'));
    await secondWrite.done;
    expect(rolledBack).toEqual(['B']);
  });
});
