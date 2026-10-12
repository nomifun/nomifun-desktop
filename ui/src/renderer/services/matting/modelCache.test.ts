/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { ensureMattingModel } from './modelCache';

const cachesDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'caches');
const initialTrust = window.__nomiLocalTrust;

afterEach(() => {
  mock.restore();
  if (cachesDescriptor) Object.defineProperty(globalThis, 'caches', cachesDescriptor);
  else Reflect.deleteProperty(globalThis, 'caches');
  if (initialTrust === undefined) delete window.__nomiLocalTrust;
  else window.__nomiLocalTrust = initialTrust;
});

test('authenticated preparation and wizard share a retryable flight, then reuse a cached model', async () => {
  let cached = false;
  Object.defineProperty(globalThis, 'caches', { configurable: true, value: {
    open: async () => ({
      match: async () => cached ? new Response(null, { headers: { 'content-length': String(8 * 1024 * 1024) } }) : undefined,
    }),
  } });
  window.__nomiLocalTrust = 'test-matting-local-trust';
  let failDownload!: (error: Error) => void;
  const fetching = spyOn(globalThis, 'fetch').mockImplementation(() =>
    new Promise<Response>((_resolve, reject) => { failDownload = reject; })
  );
  const background = ensureMattingModel();
  const wizard = ensureMattingModel(() => {});
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(fetching).toHaveBeenCalledTimes(1);
  expect(fetching.mock.calls[0]?.[1]).toEqual({
    headers: { 'x-nomi-local-trust': 'test-matting-local-trust' }, credentials: 'include',
  });
  failDownload(new Error('offline'));
  const failures = await Promise.allSettled([background, wizard]);
  expect(failures.every(result => result.status === 'rejected')).toBe(true);

  // Failure clears the shared flight. A model already present on retry requires
  // no request, and progress cleanup does not create an unhandled rejection.
  cached = true;
  await ensureMattingModel();
  expect(fetching).toHaveBeenCalledTimes(1);
});
