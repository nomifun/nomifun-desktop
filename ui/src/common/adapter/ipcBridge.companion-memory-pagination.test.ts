/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { companion } from './ipcBridge';
import { parseCompanionId, parseCompanionMemoryId } from '../types/ids';

const source = readFileSync(new URL('./ipcBridge.ts', import.meta.url), 'utf8');

const COMPANION_ID = parseCompanionId('0190f5fe-7c00-7a00-8000-000000000001');
const MEMORY_ID = parseCompanionMemoryId('0190f5fe-7c00-7a00-8000-000000000002');
const realFetch = globalThis.fetch;

describe('companion memory pagination bridge', () => {
  test('exposes paged memory items with a filtered total', () => {
    expect(source.includes('export interface ICompanionMemoryPage')).toBe(true);
    expect(source.includes('items: ICompanionMemory[];')).toBe(true);
    expect(source.includes('total: number;')).toBe(true);
    expect(source.includes('listMemories: withResponseMap(')).toBe(true);
    expect(/listMemories: withResponseMap\(\s*httpGet<\s*\{ items: unknown\[\]; total: number \}/.test(source)).toBe(true);
    expect(source.includes('(raw): ICompanionMemoryPage')).toBe(true);
    expect(source.includes('raw.items.map(fromApiCompanionMemory)')).toBe(true);
  });

  test('uses memory_id for the memory wire identity and rejects legacy id adapters', () => {
    expect(source.includes('memory_id: CompanionMemoryId;')).toBe(true);
    expect(source.includes('memory_id: parseCompanionMemoryId(value.memory_id)')).toBe(true);
    expect(source.includes('{ memory_id: CompanionMemoryId;')).toBe(true);
    expect(source.includes('{ id: CompanionMemoryId;')).toBe(false);
    expect(source.includes('value.id')).toBe(false);
  });

  /**
   * Ownership is enforced in the store, which means the ASKING companion has to
   * be on the wire for every mutation — including the two that carry it somewhere
   * other than a JSON body (DELETE's query string) where a wrong parameter name
   * would still typecheck. Without it the backend answers 400.
   */
  test('every memory mutation carries the asking companion', async () => {
    const calls: Array<{ method: string; url: string; body?: Record<string, unknown> }> = [];
    try {
      globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        calls.push({
          method: init?.method ?? 'GET',
          url,
          body: typeof init?.body === 'string' ? JSON.parse(init.body) : undefined,
        });
        // `mergeMemories` maps its response through the memory parser; everything
        // else here ignores the payload.
        const data = url.endsWith('/memories/merge')
          ? {
              memory_id: MEMORY_ID,
              kind: 'knowledge',
              content: 'x',
              tags: [],
              importance: 0.8,
              strength: 0.8,
              pinned: false,
              source: 'merge',
              status: 'active',
              created_at: 1,
              updated_at: 1,
              last_reinforced_at: 1,
              scope_kind: 'companion',
              scope_companion_id: COMPANION_ID,
            }
          : [];
        return new Response(JSON.stringify({ success: true, data }), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }) as typeof fetch;

      await companion.updateMemory.invoke({
        memory_id: MEMORY_ID,
        content: 'x',
        scope_companion_id: COMPANION_ID,
      });
      await companion.deleteMemory.invoke({ memory_id: MEMORY_ID, scope_companion_id: COMPANION_ID });
      await companion.batchMemories.invoke({
        ids: [MEMORY_ID],
        action: 'archive',
        scope_companion_id: COMPANION_ID,
      });
      await companion.memoryMergeSuggestions.invoke({ scope_companion_id: COMPANION_ID });
      await companion.mergeMemories.invoke({
        group: [MEMORY_ID],
        merged_content: 'x',
        kind: 'knowledge',
        scope_companion_id: COMPANION_ID,
      });

      expect(calls.map((call) => call.method)).toEqual(['PUT', 'DELETE', 'POST', 'POST', 'POST']);
      // DELETE has no body, so the owner rides in the query string.
      expect(calls[1]?.url.endsWith(`/api/companion/memories/${MEMORY_ID}?scope_companion_id=${COMPANION_ID}`)).toBe(
        true
      );
      for (const call of calls) {
        if (call.method === 'DELETE') continue;
        expect(call.body?.scope_companion_id).toBe(COMPANION_ID);
      }
    } finally {
      globalThis.fetch = realFetch;
    }
  });
});
