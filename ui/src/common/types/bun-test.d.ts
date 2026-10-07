/**
 * Load only Bun's test API: renderer code must retain browser globals, not
 * Bun's process-wide fetch/Request extensions. Keep this version aligned with
 * the test runner in ui/package.json.
 */
import 'bun-types/test';

declare module 'bun:test' {
  interface MatchersBuiltin<T> {
    // Runtime equality intentionally compares branded IDs and wire snapshots
    // against plain literals (also for negative assertions). Preserve the
    // previous test contract without casting fixtures to production types.
    toBe(expected: unknown): void;
    toEqual(expected: unknown): void;
    toStrictEqual(expected: unknown): void;
  }
}
