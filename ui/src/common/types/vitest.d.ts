/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Ambient shim for `vitest`. A handful of `*.test.ts` files import from
 * `vitest`, but the package is not installed (the project's runnable tests use
 * `bun:test`; see `bun-test.d.ts`). Without this shim every such import trips
 * TS2307 and inflates the typecheck baseline. The shim mirrors the small subset
 * of the vitest API these tests actually use.
 */
declare module 'vitest' {
  type TestFn = () => void | Promise<void>;

  interface Matchers {
    not: Matchers;
    toBe(expected: unknown): void;
    toEqual(expected: unknown): void;
    toStrictEqual(expected: unknown): void;
    toHaveLength(expected: number): void;
    toContain(expected: unknown): void;
    toBeCloseTo(expected: number, precision?: number): void;
    toBeDefined(): void;
    toBeUndefined(): void;
    toBeTruthy(): void;
    toBeFalsy(): void;
    toBeNull(): void;
    toMatchObject(expected: unknown): void;
    toThrow(expected?: unknown): void;
    toBeGreaterThan(expected: number): void;
    toBeGreaterThanOrEqual(expected: number): void;
    toBeLessThan(expected: number): void;
    toBeLessThanOrEqual(expected: number): void;
  }

  export function describe(name: string, fn: TestFn): void;
  export function it(name: string, fn: TestFn): void;
  export function test(name: string, fn: TestFn): void;
  export function expect(actual: unknown): Matchers;
  export function beforeEach(fn: TestFn): void;
  export function afterEach(fn: TestFn): void;
  export function beforeAll(fn: TestFn): void;
  export function afterAll(fn: TestFn): void;
}
