/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import model from '../../../../../crates/backend/nomifun-companion/matting-model.json';
import { createMattingModelLoader, type ModelProgress } from './modelCache';

afterEach(() => { mock.restore(); });

test('background preparation and inference share actual immutable bytes; failures remain retryable', async () => {
  let fail!: (error: Error) => void;
  const download = mock(() => new Promise<Blob>((_resolve, reject) => { fail = reject; }));
  const acquire = createMattingModelLoader(download);
  const background = acquire();
  const wizard = acquire();
  await Promise.resolve();
  expect(download).toHaveBeenCalledTimes(1);
  fail(new Error('offline'));
  expect((await Promise.allSettled([background, wizard])).every(r => r.status === 'rejected')).toBe(true);

  const bytes = new Blob(['verified model']);
  download.mockImplementation(() => Promise.resolve(bytes));
  expect(await acquire()).toBe(bytes);
  expect(await acquire()).toBe(bytes);
  expect(download).toHaveBeenCalledTimes(2);
});

test('closing a consumer removes its progress listener without cancelling shared preparation', async () => {
  let finish!: (blob: Blob) => void;
  let publish!: ModelProgress;
  const acquire = createMattingModelLoader(progress => {
    publish = progress;
    return new Promise<Blob>(resolve => { finish = resolve; });
  });
  const progress = mock(() => {});
  const cancellation = new AbortController();
  const background = acquire();
  const wizard = acquire(progress, cancellation.signal);
  await Promise.resolve();
  publish(1, 2);
  expect(progress).toHaveBeenCalledTimes(1);
  cancellation.abort();
  await expect(wizard).rejects.toHaveProperty('name', 'AbortError');
  publish(2, 2);
  expect(progress).toHaveBeenCalledTimes(1);
  const bytes = new Blob(['model']);
  finish(bytes);
  expect(await background).toBe(bytes);
  expect(await acquire()).toBe(bytes);
});

test('one disposed progress view cannot break the model shared with another view', async () => {
  const acquire = createMattingModelLoader(async progress => {
    progress(1, 1);
    return new Blob(['model']);
  });
  const healthy = mock(() => {});
  const broken = acquire(() => { throw new Error('disposed'); });
  const other = acquire(healthy);
  expect(await broken).toBe(await other);
  expect(healthy).toHaveBeenCalledWith(1, 1);
});

test('acquisition works without Cache Storage and requests the approved backend identity', async () => {
  const cacheDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'caches');
  Object.defineProperty(globalThis, 'caches', { configurable: true, get: () => { throw new Error('storage unavailable'); } });
  try {
    const fetching = spyOn(globalThis, 'fetch').mockResolvedValue(new Response(new Uint8Array(model.size), {
      headers: { 'Content-Type': 'application/octet-stream' },
    }));
    const acquire = createMattingModelLoader();
    const blob = await acquire();
    expect(blob.size).toBe(model.size);
    expect(await acquire()).toBe(blob);
    expect(fetching).toHaveBeenCalledTimes(1);
    expect(String(fetching.mock.calls[0]?.[0])).toContain(`sha256=${model.sha256}`);
    expect(fetching.mock.calls[0]?.[1]?.credentials).toBe('same-origin');
  } finally {
    if (cacheDescriptor) Object.defineProperty(globalThis, 'caches', cacheDescriptor);
    else Reflect.deleteProperty(globalThis, 'caches');
  }
});

test('a plausible length header cannot make a truncated model ready; the next attempt can recover', async () => {
  const fetching = spyOn(globalThis, 'fetch').mockResolvedValue(new Response(new Uint8Array(2), {
    headers: { 'Content-Type': 'application/octet-stream', 'Content-Length': String(model.size) },
  }));
  const acquire = createMattingModelLoader();
  await expect(acquire()).rejects.toThrow(`expected ${model.size} bytes, received 2`);
  fetching.mockResolvedValue(new Response(new Uint8Array(model.size), {
    headers: { 'Content-Type': 'application/octet-stream' },
  }));
  expect((await acquire()).size).toBe(model.size);
  expect(fetching).toHaveBeenCalledTimes(2);
});
