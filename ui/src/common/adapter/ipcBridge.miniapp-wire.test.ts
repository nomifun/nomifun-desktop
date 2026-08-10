/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { InvalidEntityIdError } from '@/common/types/ids';
import { miniapps, type IApiMiniApp } from './ipcBridge';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');
const MINIAPP_ID = '0190f5fe-7c00-7a00-8000-0000000000b1';
const CONVERSATION_ID = '0190f5fe-7c00-7a00-8000-0000000000b2';
const realFetch = globalThis.fetch;

const rawMiniApp = (miniappId: unknown) => ({
  miniapp_id: miniappId,
  name: 'Pomodoro',
  description: 'A timer with reminders',
  icon: '⏱️',
  source_conversation_id: CONVERSATION_ID,
  html_size: 4_096,
  created_at: 1,
  updated_at: 2,
});

function respondWith(data: unknown): void {
  globalThis.fetch = (() =>
    Promise.resolve(
      new Response(JSON.stringify({ success: true, data }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      })
    )) as unknown as typeof fetch;
}

describe('mini-app wire contract', () => {
  test('every CRUD call addresses the routes the backend mounts', () => {
    expect(source.includes("httpGet<IApiMiniApp[], void>('/api/miniapps')")).toBe(true);
    expect(source.includes("httpPost<IApiMiniApp, IApiCreateMiniApp>('/api/miniapps')")).toBe(true);
    // Detail / update / delete share the `{miniapp_id}` capture on the same prefix.
    expect(source.split('`/api/miniapps/${p.miniapp_id}`').length - 1).toBe(3);
    expect(source.includes('delete: httpDelete<boolean, { miniapp_id: MiniAppId }>')).toBe(true);
  });

  test('the wire shape is snake_case and never carries the HTML body', () => {
    expect(source.includes('export interface IApiMiniApp')).toBe(true);
    for (const key of [
      'miniapp_id: MiniAppId',
      'name: string',
      'description: string',
      'icon: string | null',
      'source_conversation_id: string | null',
      'html_size: number',
      'created_at: number',
      'updated_at: number',
    ]) {
      expect(source.includes(key)).toBe(true);
    }
    // The document itself only ever travels on create/update requests and back
    // out through the unauthenticated serve route.
    expect(source.includes('export interface IApiCreateMiniApp')).toBe(true);
    expect(source.includes('html: string')).toBe(true);
    expect(source.includes('html?: string')).toBe(true);
  });

  test('one mapper brands miniapp_id for every arrival path', () => {
    expect(source.includes('const fromApiMiniApp')).toBe(true);
    expect(source.includes('miniapp_id: parseMiniAppId(value.miniapp_id)')).toBe(true);
    // list + get + create + update all route through the single mapper.
    expect(source.split('fromApiMiniApp').length - 1).toBeGreaterThanOrEqual(5);
  });

  test('list rows are branded at the boundary and a prefixed id is rejected', async () => {
    try {
      respondWith([rawMiniApp(MINIAPP_ID)]);
      const rows: IApiMiniApp[] = await miniapps.list.invoke();
      expect(rows[0]?.miniapp_id).toBe(MINIAPP_ID);
      expect(rows[0]?.html_size).toBe(4_096);
      expect(rows[0]?.source_conversation_id).toBe(CONVERSATION_ID);

      respondWith([rawMiniApp(`miniapp_${MINIAPP_ID}`)]);
      let error: unknown;
      try {
        await miniapps.list.invoke();
      } catch (caught) {
        error = caught;
      }
      expect(error instanceof InvalidEntityIdError).toBe(true);
    } finally {
      globalThis.fetch = realFetch;
    }
  });

  test('a missing mini-app survives the detail mapper as null', async () => {
    try {
      respondWith(null);
      expect(await miniapps.get.invoke({ miniapp_id: MINIAPP_ID as IApiMiniApp['miniapp_id'] })).toBe(null);
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
