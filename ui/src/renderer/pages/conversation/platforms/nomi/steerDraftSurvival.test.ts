/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

/**
 * Steering has no durable channel of its own: the request either reaches the
 * running turn or the interjection is gone. The draft is cleared before the
 * request resolves (so an in-flight steer cannot overwrite newer typing), and
 * the failure used to be swallowed — so an offline click destroyed the text with
 * only an error toast. The observed loss was
 * `DRAFT_SHOULD_SURVIVE_OFFLINE_20260816`.
 *
 * The fix diverts a failed interjection into the same persisted command queue
 * the normal send path uses when busy. These tests model that contract.
 */

interface QueuedCommand {
  input: string;
  files: string[];
}

interface SteerBox {
  input: string;
  files: string[];
  queue: QueuedCommand[];
}

/** Mirrors onSteerHandler: clear eagerly, enqueue on failure. */
const runSteer = async (
  box: SteerBox,
  steer: () => Promise<void>,
  typeWhileInFlight?: string
): Promise<SteerBox> => {
  const state: SteerBox = { ...box, queue: [...box.queue] };
  const sent = { input: state.input, files: state.files };

  state.input = '';
  state.files = [];

  const pending = steer();
  if (typeWhileInFlight !== undefined) state.input = typeWhileInFlight;

  try {
    await pending;
  } catch {
    state.queue.push({ input: sent.input, files: sent.files });
  }
  return state;
};

const box = (): SteerBox => ({
  input: 'DRAFT_SHOULD_SURVIVE_OFFLINE_20260816',
  files: ['a.ts', 'b.ts'],
  queue: [],
});

const offline = async () => {
  throw new Error('Failed to fetch');
};

describe('steer draft survival', () => {
  test('a successful steer clears the box and queues nothing', async () => {
    const state = await runSteer(box(), async () => {});
    expect(state.input).toBe('');
    expect(state.files).toEqual([]);
    expect(state.queue).toEqual([]);
  });

  test('a failed steer preserves the text and attachments in the queue', async () => {
    const state = await runSteer(box(), offline);
    expect(state.queue).toEqual([
      { input: 'DRAFT_SHOULD_SURVIVE_OFFLINE_20260816', files: ['a.ts', 'b.ts'] },
    ]);
  });

  test('the queued copy is what was sent, not what the box holds afterwards', async () => {
    // The box is cleared before the request resolves, so reading current state
    // at failure time would queue an empty message.
    const state = await runSteer(box(), offline, 'text typed after clicking');
    expect(state.queue[0]?.input).toBe('DRAFT_SHOULD_SURVIVE_OFFLINE_20260816');
    expect(state.input).toBe('text typed after clicking');
  });

  test('every failure mode is covered, not just offline', async () => {
    for (const failure of [
      new Error('Failed to fetch'),
      new Error('timeout'),
      Object.assign(new Error('conflict'), { status: 409 }),
      Object.assign(new Error('unavailable'), { status: 503 }),
      Object.assign(new Error('attachment rejected'), { status: 413 }),
    ]) {
      const state = await runSteer(box(), async () => {
        throw failure;
      });
      expect(state.queue).toHaveLength(1);
      expect(state.queue[0]?.input).toBe('DRAFT_SHOULD_SURVIVE_OFFLINE_20260816');
    }
  });
});
