/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';
import { createGenerator } from 'unocss';
import cronEn from '@renderer/services/i18n/locales/en-US/cron.json';
import cronZh from '@renderer/services/i18n/locales/zh-CN/cron.json';
import unoConfig from '../../../../../uno.config';
import * as scheduledTaskLayout from './scheduledTaskLayout';

const pageSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');

const uno = await createGenerator(unoConfig);

/**
 * Asserts INTENT, not a literal class name: the focused search pill must end up
 * with a `border-color` the browser can actually parse.
 *
 * This replaces a string assertion on `!border-[rgb(var(--primary-6))]`, which
 * UnoCSS compiles to `rgb(var(--primary-6) / var(--un-border-opacity))`. Because
 * the ramp variables are comma-separated triplets that expands to
 * `rgb(232, 23, 74 / 1)` — invalid, so the browser threw the declaration away and
 * the focused pill kept its unfocused border. Compiling the utility catches that;
 * matching its name could not.
 */
async function expectRealBorderColor(utility: string): Promise<void> {
  const { css } = await uno.generate(utility, { preflights: false });
  const declaration = css.match(/border(?:-[a-z]+)?-color\s*:\s*([^;}!]+)/)?.[1]?.trim() ?? '';

  expect(css.trim()).not.toBe('');
  expect(declaration).not.toBe('');
  expect(/\/\s*var\(--un-/.test(declaration)).toBe(false);
  expect(['transparent', 'currentColor', 'inherit', 'unset', 'initial'].includes(declaration)).toBe(false);
}

test('keeps responsive utility classes in JSX instead of runtime exports', () => {
  const layout = scheduledTaskLayout as Record<string, unknown>;

  expect(layout.getScheduledTaskLayout).toBeUndefined();
  expect(layout.SCHEDULED_TASK_LIST_CLASS_NAMES).toBeUndefined();
  expect(layout.SCHEDULED_TASK_ROW_CLASS_NAMES).toBeUndefined();
});

test('defines five readable desktop columns', () => {
  expect((scheduledTaskLayout as Record<string, unknown>).DESKTOP_SCHEDULED_TASK_COLUMNS).toBe(
    'minmax(0,1.6fr) minmax(150px,1.1fr) minmax(84px,auto) minmax(120px,1fr) 44px'
  );
});

test('provides localized desktop-only column labels', () => {
  expect((cronZh.page as Record<string, unknown>).list).toEqual({
    task: '任务标题',
    status: '任务状态',
    action: '启停',
  });
  expect((cronEn.page as Record<string, unknown>).list).toEqual({
    task: 'Task',
    status: 'Status',
    action: 'On / off',
  });
});

test('uses compact desktop task rows', () => {
  expect(pageSource.includes('md:min-h-40px')).toBe(true);
  expect(pageSource.includes('md:py-4px')).toBe(true);
  expect(pageSource.includes('md:min-h-44px')).toBe(false);
  expect(pageSource.includes('md:py-6px')).toBe(false);
  expect(pageSource.includes('md:min-h-48px')).toBe(false);
  expect(pageSource.includes('md:py-8px')).toBe(false);
  expect(pageSource.includes('md:min-h-68px')).toBe(false);
  expect(pageSource.includes('md:py-14px')).toBe(false);
});

test('removes only the desktop perimeter and keeps internal dividers', () => {
  expect(pageSource.includes('rounded-t-12px')).toBe(false);
  expect(pageSource.includes('md:rounded-b-12px')).toBe(false);
  expect(pageSource.includes('md:divide-y')).toBe(true);
  expect(pageSource.includes('border-b-[var(--color-border-2)]')).toBe(true);
});

test('keeps desktop table surfaces transparent', () => {
  const desktopHeaderClass =
    pageSource.match(/className='hidden items-center gap-16px[^']*md:grid'/)?.[0] ?? '';
  const desktopListClass =
    pageSource.match(/className='grid w-full grid-cols-1 items-start gap-8px[^']*md:divide-\[var\(--color-border-2\)\]'/)?.[0] ?? '';
  const desktopRowClass =
    pageSource.match(/className='group flex cursor-pointer flex-col[^']*md:hover:shadow-none'/)?.[0] ?? '';

  expect(desktopHeaderClass.includes('bg-fill-2')).toBe(false);
  expect(desktopListClass.includes('md:bg-fill-1')).toBe(false);
  expect(desktopRowClass.includes('bg-fill-1')).toBe(true);
  expect(desktopRowClass.includes('md:bg-transparent')).toBe(true);
  expect(desktopHeaderClass.includes('border-b-[var(--color-border-2)]')).toBe(true);
  expect(desktopListClass.includes('md:divide-y')).toBe(true);
});

test('styles the scheduled task search as a bordered pill', async () => {
  const searchClass =
    pageSource.match(/<Input\.Search[\s\S]*?className='([^']+)'[\s\S]*?\/>/)?.[1] ?? '';
  const searchClasses = searchClass.split(/\s+/);

  expect(searchClasses.includes('[&_.arco-input-inner-wrapper]:!rounded-full')).toBe(true);
  expect(searchClasses.includes('[&_.arco-input-inner-wrapper]:!border')).toBe(true);
  expect(searchClasses.includes('[&_.arco-input-inner-wrapper]:!border-solid')).toBe(true);
  expect(searchClasses.includes('[&_.arco-input-inner-wrapper]:!border-[var(--color-border-2)]')).toBe(true);
  expect(searchClasses.includes('[&_.arco-input-inner-wrapper:hover]:!border-[var(--color-border-3)]')).toBe(true);

  // The focused pill must actually change colour, whatever utility spells it.
  const focusBorderUtility = searchClasses.find(
    (token) => token.startsWith('[&_.arco-input-inner-wrapper-focus]:') && token.includes('border-')
  );
  expect(focusBorderUtility).toBeDefined();
  await expectRealBorderColor(focusBorderUtility as string);
});

test('places localized status filters below the search input', () => {
  expect((cronZh.page as Record<string, unknown>).statusFilter).toEqual({
    label: '任务状态筛选',
    all: '全部',
    active: '已启动',
    paused: '已暂停',
  });
  expect(pageSource.includes("(['all', 'active', 'paused'] as const)")).toBe(true);
  expect(pageSource.includes('aria-pressed={selected}')).toBe(true);
  expect(pageSource.includes('filterCronJobsByStatus')).toBe(true);
  expect(pageSource.includes('appearance-none !border-0 !outline-none !shadow-none')).toBe(true);
  expect(pageSource.includes('rounded-8px px-9px py-4px text-13px leading-18px')).toBe(true);
});

test('uses a desktop more menu without changing the mobile switch contract', () => {
  expect(pageSource.includes("t('cron.page.list.action')")).toBe(false);
  expect(pageSource.includes("import ScheduledTaskActions from './ScheduledTaskActions'")).toBe(true);
  expect(pageSource.includes('deleteJob')).toBe(true);
  expect(pageSource.includes('<ScheduledTaskActions')).toBe(true);

  const mobileSwitchBlock =
    pageSource.match(/className='shrink-0 md:hidden'[\s\S]*?<Switch[\s\S]*?handleToggleEnabled\(job\)/)?.[0] ?? '';
  expect(Boolean(mobileSwitchBlock)).toBe(true);
});
