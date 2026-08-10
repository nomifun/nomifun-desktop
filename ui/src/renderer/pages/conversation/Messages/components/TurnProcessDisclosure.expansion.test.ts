import { describe, expect, test } from 'bun:test';

import { shouldResetTurnProcessDisclosureExpansion } from './TurnProcessDisclosure';

describe('TurnProcessDisclosure expansion state', () => {
  test('resets the same turn when it finishes so the process collapses', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: false },
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true }
      )
    ).toBe(true);
  });

  test('preserves manual expansion while the turn lifecycle is unchanged', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: false },
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: false }
      )
    ).toBe(false);
  });

  test('resets when a new turn disclosure replaces the current one', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: false },
        { itemId: 'turn-disclosure-2', hasProcessItems: true, defaultCollapsed: false }
      )
    ).toBe(true);
  });

  test('resets when process items first arrive for the current turn', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: false, defaultCollapsed: false },
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: false }
      )
    ).toBe(true);
  });
});
