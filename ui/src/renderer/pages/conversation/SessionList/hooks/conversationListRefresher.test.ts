/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId } from '@/common/types/ids';
import { createConversationListRefresher } from './conversationListRefresher';

const conversation = (n: number) => ({
  id: parseConversationId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0')),
  name: 'conversation ' + n,
}) as TChatConversation;

const flush = async () => {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
};

const fixture = (initial: TChatConversation[] = []) => {
  const pending: Array<{
    resolve: (items: TChatConversation[]) => void;
    reject: (error: unknown) => void;
  }> = [];
  const errors: unknown[] = [];
  const applied: TChatConversation[][] = [];
  let conversations = initial;
  const refresher = createConversationListRefresher({
    load: () => new Promise((resolve, reject) => pending.push({ resolve, reject })),
    apply: (items) => {
      conversations = items;
      applied.push(items);
    },
    remove: (id) => { conversations = conversations.filter((item) => item.id !== id); },
    onError: (error) => { errors.push(error); },
  });
  return { ...refresher, pending, errors, applied, rows: () => conversations };
};

describe('conversation history snapshot refresh', () => {
  test('serializes reads and combines a burst into one follow-up without starving published snapshots', async () => {
    const f = fixture();
    const refresh = f.refresh();
    await flush();
    for (let i = 0; i < 100; i += 1) expect(f.refresh()).toBe(refresh);
    expect(f.pending.length).toBe(1);

    f.pending[0]!.resolve([conversation(1)]);
    await flush();
    expect(f.rows()).toEqual([conversation(1)]);
    expect(f.pending.length).toBe(2);

    // Ongoing turn events still allow the current read to publish before the next.
    void f.refresh();
    f.pending[1]!.resolve([conversation(2)]);
    await flush();
    expect(f.rows()).toEqual([conversation(2)]);
    expect(f.pending.length).toBe(3);
    f.pending[2]!.resolve([conversation(3)]);
    await refresh;
    expect(f.applied).toEqual([[conversation(1)], [conversation(2)], [conversation(3)]]);
  });

  test('local and authoritative deletion cannot be resurrected by an older in-flight snapshot', async () => {
    const f = fixture([conversation(1), conversation(2), conversation(3)]);
    const refresh = f.refresh();
    await flush();
    void f.deleted(conversation(1).id);
    void f.deleted(conversation(2).id);
    void f.deleted(conversation(1).id); // The local receipt and WebSocket event may both arrive.
    expect(f.rows()).toEqual([conversation(3)]);

    f.pending[0]!.resolve([conversation(1), conversation(2), conversation(3)]);
    await flush();
    expect(f.rows()).toEqual([conversation(3)]);
    expect(f.pending.length).toBe(2);
    f.pending[1]!.resolve([conversation(3)]);
    await refresh;
    expect(f.applied.every((rows) => rows.every((row) => row.id === conversation(3).id))).toBe(true);
  });

  test('failed refresh keeps the last trusted list, including deletions already acknowledged', async () => {
    const f = fixture([conversation(1), conversation(2)]);
    const refresh = f.refresh();
    await flush();
    void f.deleted(conversation(1).id);
    f.pending[0]!.reject('offline');
    await flush();
    expect(f.rows()).toEqual([conversation(2)]);
    f.pending[1]!.reject('still offline');
    await refresh;
    expect(f.rows()).toEqual([conversation(2)]);
    expect(f.applied).toEqual([]);
    expect(f.errors).toEqual(['offline', 'still offline']);

    const recovery = f.refresh();
    await flush();
    f.pending[2]!.resolve([conversation(2), conversation(3)]);
    await recovery;
    expect(f.rows()).toEqual([conversation(2), conversation(3)]);
  });

  test('a queued refresh still runs after a failed read', async () => {
    const f = fixture([conversation(1)]);
    const refresh = f.refresh();
    await flush();
    void f.refresh();
    f.pending[0]!.reject('old read failed');
    await flush();
    expect(f.rows()).toEqual([conversation(1)]);
    expect(f.pending.length).toBe(2);
    f.pending[1]!.resolve([conversation(2)]);
    await refresh;
    expect(f.rows()).toEqual([conversation(2)]);
  });

  test('deletion overlays belong to the in-flight read and do not become permanent history', async () => {
    const f = fixture([conversation(1)]);
    const refresh = f.refresh();
    await flush();
    void f.deleted(conversation(1).id);
    f.pending[0]!.resolve([conversation(1)]);
    await flush();
    expect(f.rows()).toEqual([]);
    // The next request starts after deletion; its durable snapshot is authoritative.
    f.pending[1]!.resolve([conversation(1)]);
    await refresh;
    expect(f.rows()).toEqual([conversation(1)]);
  });

  test('a synchronously failing loader does not wedge future refreshes', async () => {
    let attempts = 0;
    let rows: TChatConversation[] = [];
    const errors: unknown[] = [];
    const f = createConversationListRefresher({
      load: () => {
        attempts += 1;
        if (attempts === 1) throw new Error('invalid response');
        return Promise.resolve([conversation(1)]);
      },
      apply: (items) => { rows = items; },
      remove: () => {},
      onError: (error) => { errors.push(error); },
    });
    await f.refresh();
    await f.refresh();
    expect(attempts).toBe(2);
    expect(errors.length).toBe(1);
    expect(rows).toEqual([conversation(1)]);
  });
});
