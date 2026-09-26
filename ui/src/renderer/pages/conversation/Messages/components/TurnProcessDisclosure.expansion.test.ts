import { describe, expect, test } from 'bun:test';

import { shouldResetTurnProcessDisclosureExpansion } from './TurnProcessDisclosure';

describe('TurnProcessDisclosure expansion state', () => {
  test('resets to the collapsed default when a turn finishes', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: false, running: true },
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true, running: false }
      )
    ).toBe(true);
  });

  test('preserves manual expansion while the turn lifecycle is unchanged', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true, running: true },
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true, running: true }
      )
    ).toBe(false);
  });

  test('resets when a new turn disclosure replaces the current one', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true, running: true },
        { itemId: 'turn-disclosure-2', hasProcessItems: true, defaultCollapsed: true, running: true }
      )
    ).toBe(true);
  });

  test('resets when process items first arrive for the current turn', () => {
    expect(
      shouldResetTurnProcessDisclosureExpansion(
        { itemId: 'turn-disclosure-1', hasProcessItems: false, defaultCollapsed: true, running: true },
        { itemId: 'turn-disclosure-1', hasProcessItems: true, defaultCollapsed: true, running: true }
      )
    ).toBe(true);
  });
});
