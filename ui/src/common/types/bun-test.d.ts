/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

declare module 'bun:test' {
  type TestFn = () => void | Promise<void>;

  interface Matchers {
    not: Matchers;
    toBe(expected: unknown): void;
    toEqual(expected: unknown): void;
    toHaveLength(expected: number): void;
    toBeCloseTo(expected: number, precision?: number): void;
    toBeDefined(): void;
    toBeUndefined(): void;
    toBeTruthy(): void;
    toBeFalsy(): void;
    toBeNull(): void;
    toMatchObject(expected: unknown): void;
    toBeGreaterThan(expected: number): void;
    toBeGreaterThanOrEqual(expected: number): void;
    toBeLessThan(expected: number): void;
    toBeLessThanOrEqual(expected: number): void;
  }

  interface Test {
    (name: string, fn: TestFn): void;
    each<T>(cases: readonly T[]): (name: string, fn: (caseValue: T) => void | Promise<void>) => void;
    each<T extends readonly unknown[]>(
      cases: readonly T[]
    ): (name: string, fn: (...caseValues: T) => void | Promise<void>) => void;
  }

  export function describe(name: string, fn: TestFn): void;
  export const test: Test;
  export function expect(actual: unknown): Matchers;
}
