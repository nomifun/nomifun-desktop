/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./PresetCard.tsx', import.meta.url), 'utf8');

describe('PresetCard visual hierarchy', () => {
  test('uses the shared neutral card background and theme outline', () => {
    expect(
      source.includes(
        "'group relative flex flex-col rounded-16px border border-solid px-14px pt-14px pb-8px cursor-pointer'"
      )
    ).toBe(true);
    expect(source.includes('min-h-[214px]')).toBe(false);
    expect(source.includes("'border-[var(--color-border-2)] bg-[var(--color-bg-2)]")).toBe(true);
    expect(source.includes('hover:border-[var(--color-primary-light-4)]')).toBe(true);
  });

  test('pins the text actions to the bottom and keeps their icon and label centered without a separator', () => {
    expect(source.includes('mt-auto pt-4px flex min-h-24px items-center justify-end gap-12px')).toBe(true);
    expect(source.includes('border-t border-solid')).toBe(false);
    expect(source.includes('inline-flex items-center gap-4px leading-none text-12px')).toBe(true);
  });

  test('lets short descriptions use their natural height while clamping overflow at two lines', () => {
    expect(source.includes('WebkitLineClamp: 2')).toBe(true);
    expect(source.includes('min-h-[36px]')).toBe(false);
  });
});
