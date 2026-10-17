import { describe, expect, test } from 'bun:test';

import { shouldResetTurnProcessDisclosureExpansion } from './TurnProcessDisclosure';

describe('TurnProcessDisclosure expansion state', () => {
  test('resets to the collapsed default when a turn finishes', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: false },
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true }
      )
    ).toBe(true);
  });

  test('preserves manual expansion while the default and content presence are unchanged', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true },
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true }
      )
    ).toBe(false);
  });

  test('resets when a new turn disclosure replaces the current one', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true },
        { itemId: 'turn-disclosure-2', hasProcessItems: true, defaultCollapsed: true }
      )
    ).toBe(true);
  });

  test('resets when process items first arrive for the current turn', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: false, defaultCollapsed: true },
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true }
      )
    ).toBe(true);
  });
});
