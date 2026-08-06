/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';
import { browseDirectory, createDirectory } from './directorySelectionApi';

const realFetch = globalThis.fetch;
const realWindow = (globalThis as { window?: Window }).window;
const realDocument = (globalThis as { document?: Document }).document;

/**
 * Install a global via `Object.defineProperty` rather than plain assignment.
 * A sibling test file may have left `globalThis.window` as a read-only data
 * property, which makes `globalThis.window = …` throw in strict mode and
 * couples this file's result to suite ordering.
 */
function defineGlobal(key: 'window' | 'document', value: unknown): void {
  Object.defineProperty(globalThis, key, { configurable: true, writable: true, value });
}

function restoreGlobal(key: 'window' | 'document', original: unknown): void {
  if (original === undefined) {
    Reflect.deleteProperty(globalThis, key);
    return;
  }
  defineGlobal(key, original);
}

function installWebUiGlobals(csrfToken = ''): void {
  defineGlobal('window', {
    location: {
      origin: 'http://192.168.3.68:25808',
      protocol: 'http:',
      host: '192.168.3.68:25808',
      pathname: '/#/guid',
      hash: '#/guid',
    },
    dispatchEvent: () => true,
  });
  defineGlobal('document', {
    cookie: csrfToken ? `nomifun-csrf-token=${csrfToken}` : '',
  });
}

afterEach(() => {
  try {
    globalThis.fetch = realFetch;
  } finally {
    restoreGlobal('window', realWindow);
    restoreGlobal('document', realDocument);
  }
});

describe('directorySelectionApi WebUI request security', () => {
  test('createDirectory carries the double-submit CSRF token through the shared bridge', async () => {
    installWebUiGlobals('csrf-value');
    let capturedUrl = '';
    let capturedInit: RequestInit | undefined;
    globalThis.fetch = ((url: string | URL | Request, init?: RequestInit) => {
      capturedUrl = String(url);
      capturedInit = init;
      return Promise.resolve(
        new Response(
          JSON.stringify({
            success: true,
            data: { name: 'nomi', path: 'C:\\study\\nomi', isDirectory: true, isFile: false },
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } }
        )
      );
    }) as typeof fetch;

    const created = await createDirectory('C:\\study', 'nomi');
    const headers = new Headers(capturedInit?.headers);

    expect(capturedUrl).toBe('/api/fs/directory');
    expect(capturedInit?.method).toBe('POST');
    expect(headers.get('content-type')).toBe('application/json');
    expect(headers.get('x-csrf-token')).toBe('csrf-value');
    expect(JSON.parse(String(capturedInit?.body))).toEqual({ parentPath: 'C:\\study', name: 'nomi' });
    expect(created.path).toBe('C:\\study\\nomi');
  });

  test('createDirectory self-heals once when the backend refreshes a missing CSRF cookie', async () => {
    installWebUiGlobals();
    const csrfHeaders: Array<string | null> = [];
    globalThis.fetch = ((_url: string | URL | Request, init?: RequestInit) => {
      const headers = new Headers(init?.headers);
      csrfHeaders.push(headers.get('x-csrf-token'));
      if (csrfHeaders.length === 1) {
        (globalThis as { document?: { cookie: string } }).document!.cookie =
          'nomifun-csrf-token=fresh-token';
        return Promise.resolve(
          new Response(
            JSON.stringify({ success: false, error: 'Forbidden: CSRF token validation failed', code: 'FORBIDDEN' }),
            { status: 403, headers: { 'Content-Type': 'application/json' } }
          )
        );
      }
      return Promise.resolve(
        new Response(
          JSON.stringify({
            success: true,
            data: { name: 'nomi', path: 'C:\\study\\nomi', isDirectory: true, isFile: false },
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } }
        )
      );
    }) as typeof fetch;

    await createDirectory('C:\\study', 'nomi');

    expect(csrfHeaders).toEqual([null, 'fresh-token']);
  });

  test('browseDirectory remains a cache-bypassing GET with encoded picker parameters', async () => {
    installWebUiGlobals('csrf-value');
    let capturedUrl = '';
    let capturedInit: RequestInit | undefined;
    globalThis.fetch = ((url: string | URL | Request, init?: RequestInit) => {
      capturedUrl = String(url);
      capturedInit = init;
      return Promise.resolve(
        new Response(
          JSON.stringify({ success: true, data: { items: [], currentPath: 'C:\\study', canGoUp: true } }),
          { status: 200, headers: { 'Content-Type': 'application/json' } }
        )
      );
    }) as typeof fetch;

    await browseDirectory('C:\\study folder', false);

    expect(capturedUrl).toBe('/api/fs/browse?path=C%3A%5Cstudy%20folder&showFiles=false');
    expect(capturedInit?.method).toBe('GET');
    expect(capturedInit?.cache).toBe('no-store');
    expect(new Headers(capturedInit?.headers).has('x-csrf-token')).toBe(false);
  });
});
