/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import {
  AUTH_EXPIRED_EVENT,
  BackendHttpError,
  BackendRequestError,
  httpBlobRequest,
  httpRequest,
  isHandledAuthExpiredHttpError,
} from './httpBridge';

const realFetch = globalThis.fetch;
const realWindow = globalThis.window;
const realDocument = globalThis.document;
const binaryHeaders = { 'Content-Type': 'application/octet-stream' };
const jsonHeaders = { 'Content-Type': 'application/json' };

async function rejectedWith<T extends Error>(
  pending: Promise<unknown>,
  kind: abstract new (...args: never[]) => T,
): Promise<T> {
  const result: unknown = await pending.catch(error => error);
  expect(result).toBeInstanceOf(kind);
  if (!(result instanceof kind)) throw new Error('Request did not reject with the expected error');
  return result;
}

function installBrowser(desktop: boolean, emitted: string[] = []) {
  const location = { pathname: '/nomi', hash: '' };
  Object.assign(globalThis, {
    window: {
      location,
      dispatchEvent: (event: Event) => { emitted.push(event.type); return true; },
      ...(desktop ? { __backendPort: 25808, __nomiLocalTrust: 'desktop-trust' } : {}),
    },
    document: { cookie: 'nomifun-session=browser-session' },
  });
  return location;
}

afterEach(() => {
  globalThis.fetch = realFetch;
  Object.assign(globalThis, { window: realWindow, document: realDocument });
});

const readers = [
  { name: 'JSON', request: (options?: { timeoutMs?: number; signal?: AbortSignal }) => httpRequest('GET', '/api/model', undefined, options),
    response: () => new Response('{"success":true,"data":{"ready":true}}', { headers: jsonHeaders }), contentType: 'application/json' },
  { name: 'binary', request: (options?: { timeoutMs?: number; signal?: AbortSignal }) => httpBlobRequest('/api/model', options),
    response: () => new Response(new Uint8Array([1, 2, 3]), { headers: binaryHeaders }), contentType: 'application/octet-stream' },
];

describe('shared JSON and binary backend transport', () => {
  for (const reader of readers) {
    test(`${reader.name} uses desktop trust with same-origin credentials and bypasses HTTP cache`, async () => {
      installBrowser(true);
      let capturedUrl: string | undefined;
      let capturedInit: RequestInit | undefined;
      globalThis.fetch = ((url, init) => {
        capturedUrl = String(url);
        capturedInit = init;
        return Promise.resolve(reader.response());
      }) as typeof fetch;
      await reader.request();
      expect(capturedUrl).toBe('http://127.0.0.1:25808/api/model');
      expect(new Headers(capturedInit?.headers).get('x-nomi-local-trust')).toBe('desktop-trust');
      expect(capturedInit?.credentials).toBe('same-origin');
      expect(capturedInit?.cache).toBe('no-store');
    });

    test(`${reader.name} keeps WebUI requests same-origin so the browser can send its session cookie`, async () => {
      installBrowser(false);
      let capturedUrl: string | undefined;
      let capturedInit: RequestInit | undefined;
      globalThis.fetch = ((url, init) => {
        capturedUrl = String(url);
        capturedInit = init;
        return Promise.resolve(reader.response());
      }) as typeof fetch;
      await reader.request();
      expect(capturedUrl).toBe('/api/model');
      expect(capturedInit?.credentials).toBe('same-origin');
      expect(new Headers(capturedInit?.headers).has('x-nomi-local-trust')).toBe(false);
    });

    test(`${reader.name} preserves structured 401 and 403 errors`, async () => {
      installBrowser(true);
      for (const status of [401, 403]) {
        globalThis.fetch = (() => Promise.resolve(new Response(
          JSON.stringify({ success: false, code: 'FORBIDDEN', error: 'access denied', details: { phase: 'authorization' } }),
          { status, headers: jsonHeaders },
        ))) as typeof fetch;
        const error = await rejectedWith(reader.request(), BackendHttpError);
        expect(error.status).toBe(status);
        expect(error.code).toBe('FORBIDDEN');
        expect(error.details).toEqual({ phase: 'authorization' });
      }
    });

    test(`${reader.name} handles expired WebUI auth through the shared event and login redirect`, async () => {
      const emitted: string[] = [];
      const location = installBrowser(false, emitted);
      globalThis.fetch = (() => Promise.resolve(new Response(
        JSON.stringify({ success: false, code: 'FORBIDDEN', error: 'Forbidden: Invalid or expired token' }),
        { status: 403, headers: jsonHeaders },
      ))) as typeof fetch;
      const error = await reader.request().catch(error => error);
      expect(isHandledAuthExpiredHttpError(error)).toBe(true);
      await new Promise(resolve => setTimeout(resolve, 0));
      expect(emitted).toContain(AUTH_EXPIRED_EVENT);
      expect(location.hash).toBe('/login');
    });

    test(`${reader.name} keeps the deadline active after response headers arrive`, async () => {
      globalThis.fetch = ((_url, init) => Promise.resolve(new Response(new ReadableStream({
        start(controller) {
          init?.signal?.addEventListener('abort', () => controller.error(new DOMException('Aborted', 'AbortError')));
        },
      }), { headers: { 'Content-Type': reader.contentType } }))) as typeof fetch;
      const error = await rejectedWith(reader.request({ timeoutMs: 10 }), BackendRequestError);
      expect(error.kind).toBe('timeout');
    });

    test(`${reader.name} preserves caller cancellation while the body is being read`, async () => {
      const caller = new AbortController();
      globalThis.fetch = ((_url, init) => Promise.resolve(new Response(new ReadableStream({
        start(controller) {
          init?.signal?.addEventListener('abort', () => controller.error(new DOMException('Aborted', 'AbortError')));
        },
      }), { headers: { 'Content-Type': reader.contentType } }))) as typeof fetch;
      const pending = rejectedWith(reader.request({ signal: caller.signal, timeoutMs: 1000 }), DOMException);
      setTimeout(() => caller.abort(), 10);
      const error = await pending;
      expect(error.name).toBe('AbortError');
      expect(error).not.toBeInstanceOf(BackendRequestError);
    });

    test(`${reader.name} diagnoses an interrupted body as a transport failure`, async () => {
      globalThis.fetch = (() => Promise.resolve(new Response(new ReadableStream({
        start(controller) { controller.error(new TypeError('connection reset')); },
      }), { headers: { 'Content-Type': reader.contentType } }))) as typeof fetch;
      const error = await rejectedWith(reader.request(), BackendRequestError);
      expect(error.kind).toBe('network');
      expect(error.message).toContain('connection reset');
    });
  }

  test('malformed JSON remains a decoder error', async () => {
    globalThis.fetch = (() => Promise.resolve(new Response('{broken json', { headers: jsonHeaders }))) as typeof fetch;
    const error = await httpRequest('GET', '/api/model').catch(error => error);
    expect(error).toBeInstanceOf(SyntaxError);
    expect(error).not.toBeInstanceOf(BackendRequestError);
  });

  test('an HTTP error body is covered by the same deadline', async () => {
    globalThis.fetch = ((_url, init) => Promise.resolve(new Response(new ReadableStream({
      start(controller) {
        init?.signal?.addEventListener('abort', () => controller.error(new DOMException('Aborted', 'AbortError')));
      },
    }), { status: 503, headers: jsonHeaders }))) as typeof fetch;
    const error = await httpBlobRequest('/api/model', { timeoutMs: 10 }).catch(error => error);
    expect(error).toBeInstanceOf(BackendRequestError);
    expect(error.kind).toBe('timeout');
  });
});

describe('bounded binary response reader', () => {
  test('returns the actual streamed bytes, reports progress, and releases the reader', async () => {
    const response = new Response(new ReadableStream({
      start(controller) {
        controller.enqueue(new Uint8Array([1, 2]));
        controller.enqueue(new Uint8Array([3, 4, 5]));
        controller.close();
      },
    }), { headers: { ...binaryHeaders, 'Content-Length': '5' } });
    globalThis.fetch = (() => Promise.resolve(response)) as typeof fetch;
    const progress: number[][] = [];
    const blob = await httpBlobRequest('/api/model', { maxBytes: 5, onProgress: (loaded, total) => progress.push([loaded, total]) });
    expect(Array.from(new Uint8Array(await blob.arrayBuffer()))).toEqual([1, 2, 3, 4, 5]);
    expect(blob.type).toBe('application/octet-stream');
    expect(progress).toEqual([[2, 5], [5, 5]]);
    expect(response.body?.locked).toBe(false);
  });

  test('supports response implementations without a readable body', async () => {
    const response = new Response(new Uint8Array([1, 2, 3]), { headers: binaryHeaders });
    Object.defineProperty(response, 'body', { value: null });
    globalThis.fetch = (() => Promise.resolve(response)) as typeof fetch;
    const progress: number[][] = [];
    const blob = await httpBlobRequest('/api/model', { maxBytes: 3, onProgress: (loaded, total) => progress.push([loaded, total]) });
    expect(blob.size).toBe(3);
    expect(progress).toEqual([[3, 3]]);
  });

  test('rejects HTML, JSON, missing, and other non-binary success content types before reading', async () => {
    for (const contentType of ['text/html', 'application/json', '', 'image/png']) {
      let cancelled = false;
      const response = new Response(new ReadableStream({ cancel() { cancelled = true; } }), {
        headers: contentType ? { 'Content-Type': contentType } : {},
      });
      globalThis.fetch = (() => Promise.resolve(response)) as typeof fetch;
      const error = await httpBlobRequest('/api/model').catch(error => error);
      expect(error).toBeInstanceOf(BackendHttpError);
      expect(error.code).toBe('NON_BINARY_RESPONSE');
      expect(cancelled).toBe(true);
      expect(response.body?.locked).toBe(false);
    }
  });

  test('rejects an oversized declared body without consuming it', async () => {
    let cancelled = false;
    let reads = 0;
    const response = new Response(new ReadableStream({
      pull(controller) { reads++; controller.enqueue(new Uint8Array([1])); },
      cancel() { cancelled = true; },
    }, { highWaterMark: 0 }), { headers: { ...binaryHeaders, 'Content-Length': '100' } });
    globalThis.fetch = (() => Promise.resolve(response)) as typeof fetch;
    const error = await httpBlobRequest('/api/model', { maxBytes: 4 }).catch(error => error);
    expect(error.code).toBe('BINARY_RESPONSE_TOO_LARGE');
    expect(reads).toBe(0);
    expect(cancelled).toBe(true);
    expect(response.body?.locked).toBe(false);
  });

  test('stops an oversized stream even when Content-Length understates its size', async () => {
    let reads = 0;
    let cancelled = false;
    const response = new Response(new ReadableStream({
      pull(controller) { reads++; controller.enqueue(new Uint8Array([1, 2, 3])); },
      cancel() { cancelled = true; },
    }, { highWaterMark: 0 }), { headers: { ...binaryHeaders, 'Content-Length': '1' } });
    globalThis.fetch = (() => Promise.resolve(response)) as typeof fetch;
    const progress: number[] = [];
    const error = await httpBlobRequest('/api/model', { maxBytes: 4, onProgress: loaded => progress.push(loaded) }).catch(error => error);
    expect(error).toBeInstanceOf(BackendHttpError);
    expect(error.code).toBe('BINARY_RESPONSE_TOO_LARGE');
    expect(error.details).toEqual({ maxBytes: 4, receivedBytes: 6 });
    expect(reads).toBe(2);
    expect(progress).toEqual([3]);
    expect(cancelled).toBe(true);
    expect(response.body?.locked).toBe(false);
  });

  test('cancels and releases the reader without mislabeling a progress callback error', async () => {
    const failure = new Error('progress subscriber failed');
    let cancelled = false;
    const response = new Response(new ReadableStream({
      pull(controller) { controller.enqueue(new Uint8Array([1])); },
      cancel() { cancelled = true; },
    }, { highWaterMark: 0 }), { headers: binaryHeaders });
    globalThis.fetch = (() => Promise.resolve(response)) as typeof fetch;
    const error = await httpBlobRequest('/api/model', { onProgress: () => { throw failure; } }).catch(error => error);
    expect(error).toBe(failure);
    expect(cancelled).toBe(true);
    expect(response.body?.locked).toBe(false);
  });
});
