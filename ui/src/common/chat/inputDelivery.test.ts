import { expect, test } from 'bun:test';
import { normalizeInputDelivery, preferInputDelivery } from './inputDelivery';

test('delivery metadata accepts only literal states and positive canonical event sequences', () => {
  for (const state of [['pending'], { state: 'pending' }, 1, null, true, 'unknown']) {
    expect(normalizeInputDelivery({ state, event_seq: 10 })).toBeUndefined();
  }
  for (const event_seq of [0, -1, 1.2, Number.NaN, Number.POSITIVE_INFINITY, Number.MAX_SAFE_INTEGER + 1, '10']) {
    expect(normalizeInputDelivery({ state: 'pending', event_seq })).toBeUndefined();
  }
  expect(normalizeInputDelivery({ state: 'blocked', reason: 'SKILL_UNAVAILABLE', event_seq: 10 }))
    .toEqual({ state: 'blocked', reason: 'SKILL_UNAVAILABLE', event_seq: 10 });
  expect(normalizeInputDelivery({ state: 'pending' })).toEqual({ state: 'pending' });
  expect(normalizeInputDelivery({ state: 'cancelled', reason: 'CANCELLED_BY_OWNER', event_seq: 12 }))
    .toEqual({ state: 'cancelled', reason: 'CANCELLED_BY_OWNER', event_seq: 12 });
});

test('a cancelled accepted input retains its terminal fact over delayed pending or blocked projections', () => {
  const cancelled = { state: 'cancelled' as const, reason: 'CANCELLED_BY_OWNER', event_seq: 20 };
  expect(preferInputDelivery({ state: 'pending' }, cancelled)).toEqual(cancelled);
  expect(preferInputDelivery({ state: 'pending', event_seq: 10 }, cancelled)).toEqual(cancelled);
  expect(preferInputDelivery({ state: 'blocked', reason: 'SKILL_UNAVAILABLE', event_seq: 19 }, cancelled)).toEqual(cancelled);
});

test('delivery ordering follows canonical sequence through blocked, pending, claimed and discarded', () => {
  const blocked = { state: 'blocked' as const, reason: 'SKILL_UNAVAILABLE', event_seq: 10 };
  const retry = { state: 'pending' as const, event_seq: 11 };
  const claimed = { state: 'claimed' as const, event_seq: 12 };
  const discarded = { state: 'discarded' as const, reason: 'CONTEXT_CLEARED', event_seq: 13 };
  expect(preferInputDelivery(retry, blocked)).toEqual(retry);
  expect(preferInputDelivery(blocked, claimed)).toEqual(claimed);
  expect(preferInputDelivery(discarded, retry)).toEqual(discarded);
  expect(preferInputDelivery({ state: 'pending' }, claimed)).toEqual(claimed);
});
