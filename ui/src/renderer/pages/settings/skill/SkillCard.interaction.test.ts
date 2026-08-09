/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

describe('SkillCard interaction ownership', () => {
  test('opens details from the card and reserves tag editing for its explicit action', () => {
    const source = readFileSync(new URL('./SkillCard.tsx', import.meta.url), 'utf8');
    const cardClick = source.indexOf('onClick={() => onOpenDetails(skill)}');
    const footerStop = source.indexOf('onClick={(e) => e.stopPropagation()}');
    const tagClick = source.indexOf('onClick={() => onEditTags(skill)}');

    expect(cardClick).toBeGreaterThanOrEqual(0);
    expect(footerStop).toBeGreaterThan(cardClick);
    expect(tagClick).toBeGreaterThan(footerStop);
    expect(source.includes('e.stopPropagation();\n              onEditTags(skill);')).toBe(true);
    expect(source.includes('border-t border-solid border-[var(--color-border-1)]')).toBe(false);
    expect(source.includes('absolute bottom-10px right-12px flex items-center justify-end')).toBe(true);
    expect(source.includes('p-12px pb-34px cursor-pointer outline-none')).toBe(true);
    expect(source.includes('text-12px leading-none text-[var(--color-text-3)] cursor-pointer hover:text-[var(--color-text-2)]')).toBe(true);
    expect(source.includes("<SettingOne theme='outline' size={13} strokeWidth={3} fill='currentColor' className='relative top-px shrink-0' />")).toBe(
      true
    );
    expect(source.includes("<span className='leading-none'>{t('settings.skillsHub.editTags'")).toBe(true);
  });

  test('separates the title and compact source badge while centering them with the icon', () => {
    const source = readFileSync(new URL('./SkillCard.tsx', import.meta.url), 'utf8');

    expect(source.includes("<div className='flex items-center gap-10px'>")).toBe(true);
    expect(source.includes("<div className='flex h-36px min-w-0 flex-1 flex-col justify-between'>")).toBe(true);
    expect(source.includes("<div className='flex h-20px min-w-0 items-center'>")).toBe(true);
    expect(source.includes("<div className='flex h-14px items-center'>")).toBe(true);
    expect(source.includes('!h-14px !text-9px !leading-12px !px-5px')).toBe(true);
    expect(source.includes("className='mt-6px text-12px leading-18px")).toBe(true);
  });
});
