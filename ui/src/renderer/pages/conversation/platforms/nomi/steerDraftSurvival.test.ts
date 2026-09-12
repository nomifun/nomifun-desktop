/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, mock, test } from 'bun:test';
import { steerOrQueue } from './steerOrQueue';

const command = () => ({
  input: 'DRAFT_SHOULD_SURVIVE_OFFLINE_20260816',
  files: ['a.ts', 'b.ts'],
});

describe('steer draft survival through the production delivery boundary', () => {
  test('a successful steer sends the snapshot and queues nothing', async () => {
    const sent = command();
    const steer = mock(async () => {});
    const enqueue = mock(() => {});
    expect(await steerOrQueue(sent, steer, enqueue)).toBe(true);
    expect(steer).toHaveBeenCalledWith(sent);
    expect(enqueue).not.toHaveBeenCalled();
  });

  test('an in-flight failure queues the submitted text and attachments', async () => {
    let draft = command();
    const sent = { input: draft.input, files: [...draft.files] };
    let reject!: (error: Error) => void;
    const delivery = new Promise<void>((_, fail) => { reject = fail; });
    const enqueue = mock(() => {});
    const pending = steerOrQueue(sent, () => delivery, enqueue);
    expect(enqueue).not.toHaveBeenCalled();
    draft = { input: 'new typing', files: ['new.ts'] };
    reject(new Error('Failed to fetch'));
    expect(await pending).toBe(false);
    expect(enqueue).toHaveBeenCalledTimes(1);
    expect(enqueue).toHaveBeenCalledWith(command());
    expect(enqueue).not.toHaveBeenCalledWith(draft);
  });

  test('all delivery failures retain the snapshot, not only offline errors', async () => {
    for (const error of [
      new Error('Failed to fetch'), new Error('timeout'),
      ...[409, 503, 413].map((status) => Object.assign(new Error('request failed'), { status })),
    ]) {
      const enqueue = mock(() => {});
      expect(await steerOrQueue(command(), async () => { throw error; }, enqueue)).toBe(false);
      expect(enqueue).toHaveBeenCalledTimes(1);
      expect(enqueue).toHaveBeenCalledWith(command());
    }
  });

  test('a queue failure remains visible to the caller', async () => {
    const error = new Error('queue storage unavailable');
    await expect(steerOrQueue(command(), async () => { throw new Error('offline'); }, () => {
      throw error;
    })).rejects.toBe(error);
  });
});
