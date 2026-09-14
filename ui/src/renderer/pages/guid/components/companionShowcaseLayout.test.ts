import { describe, expect, test } from 'bun:test';
import { fitShowcaseFigure, showcaseCapacity, visibleCompanions } from './companionShowcaseLayout';

describe('companion showcase layout', () => {
  test('fits tall, square and wide source art without changing aspect or exceeding either bound', () => {
    for (const aspect of [.2, .55, 1, 2.8, 8]) {
      const height = fitShowcaseFigure(aspect, 142, 184);
      expect(height).toBeLessThanOrEqual(184);
      expect(height * aspect).toBeLessThanOrEqual(142);
      expect(height).toBeGreaterThan(0);
    }
  });
  test('uses pane capacity and promotes a selected overflow companion without mutating the roster', () => {
    const roster = Array.from({ length: 8 }, (_, i) => ({ companion_id: String(i) }));
    expect(showcaseCapacity(360)).toBe(2);
    expect(showcaseCapacity(540)).toBe(3);
    expect(showcaseCapacity(800)).toBe(4);
    expect(visibleCompanions(roster, 2, '7').map((c) => c.companion_id)).toEqual(['0', '7']);
    expect(roster.map((c) => c.companion_id)).toEqual(['0', '1', '2', '3', '4', '5', '6', '7']);
    expect(visibleCompanions(roster, 2, 'removed').map((c) => c.companion_id)).toEqual(['0', '1']);
    expect(visibleCompanions([], 2, null)).toEqual([]);
  });
});
