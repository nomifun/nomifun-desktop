import { describe, expect, test } from 'bun:test';
import { parseMessageId } from '@/common/types/ids';
import { guidedInput, normalizeDeliveryIntent, QUEUED_INPUT } from './inputSubmission';

describe('ordinary message delivery policy', () => {
  test('ordinary submission has one immutable Queue intent', () => {
    expect(QUEUED_INPUT).toEqual({ kind: 'queue' });
    expect(Object.isFrozen(QUEUED_INPUT)).toBe(true);
  });
  test('Guiding preserves the exact observed canonical Turn', () => {
    const turn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000986');
    expect(guidedInput(turn)).toEqual({ kind: 'prefer_current', turn_id: turn });
    expect(normalizeDeliveryIntent({ kind: 'prefer_current', turn_id: 'latest' })).toBeUndefined();
    expect(normalizeDeliveryIntent({ kind: 'queue', turn_id: turn })).toBeUndefined();
  });
});
