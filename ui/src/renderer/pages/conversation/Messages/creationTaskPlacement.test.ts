import { describe, expect, test } from 'bun:test';
import { parseMessageId } from '@/common/types/ids';
import { creationTaskPlacementAfterIndices } from './creationTaskPlacement';

const turnA = parseMessageId('0190f5fe-7c00-7a00-8000-000000000101');
const requestA = parseMessageId('0190f5fe-7c00-7a00-8000-000000000102');
const turnB = parseMessageId('0190f5fe-7c00-7a00-8000-000000000103');
const requestB = parseMessageId('0190f5fe-7c00-7a00-8000-000000000104');

describe('conversation creation task placement', () => {
  test('places media after the final visible item for each owning turn', () => {
    const placements = creationTaskPlacementAfterIndices(
      [turnA, turnA, turnA, undefined, turnB, turnB],
      new Map([[turnA, requestA], [turnB, requestB]])
    );
    expect(Array.from(placements.entries())).toEqual([
      [2, { turnId: turnA, messageId: requestA }],
      [5, { turnId: turnB, messageId: requestB }],
    ]);
  });

  test('does not create placeholders for turns without creation tasks', () => {
    const placements = creationTaskPlacementAfterIndices(
      [turnA, turnB, turnB],
      new Map([[turnA, requestA]])
    );
    expect(Array.from(placements.entries())).toEqual([
      [0, { turnId: turnA, messageId: requestA }],
    ]);
  });
});
