/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import cronEn from '@renderer/services/i18n/locales/en-US/cron.json';
import cronZh from '@renderer/services/i18n/locales/zh-CN/cron.json';
import * as scheduledTaskLayout from './scheduledTaskLayout';

const { getScheduledTaskLayout } = scheduledTaskLayout;

describe('getScheduledTaskLayout', () => {
  test('keeps cards on mobile', () => {
    expect(getScheduledTaskLayout(true)).toBe('card');
  });

  test('uses horizontal rows on desktop', () => {
    expect(getScheduledTaskLayout(false)).toBe('row');
  });
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
