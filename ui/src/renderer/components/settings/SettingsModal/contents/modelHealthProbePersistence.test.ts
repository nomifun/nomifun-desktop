/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const source = readFileSync(new URL('./ModelModalContent.tsx', import.meta.url), 'utf8');

describe('task capability health UI', () => {
  test('always sends the explicitly selected task to the provider probe', () => {
    expect(source.includes('onCheck: (task: ModelTask)')).toBe(true);
    expect(source.includes('task: ModelTask) =>')).toBe(true);
    expect(source.includes('task,')).toBe(true);
    expect(source.includes('disabled={!task}')).toBe(true);
    expect(source.includes('task && void onCheck(task)')).toBe(true);
  });

  test('single-task rows probe directly while multi-task rows expose a task menu', () => {
    expect(source.includes('if (tasks.length <= 1)')).toBe(true);
    expect(source.includes('data-health-task-menu')).toBe(true);
    expect(source.includes('tasks.map((task) =>')).toBe(true);
    expect(source.includes('onCheck(task)')).toBe(true);
  });

  test('renders an independently colored and described badge for every capability', () => {
    expect(source.includes('row.capabilities.map((capability) =>')).toBe(true);
    expect(source.includes('<CapabilityHealthTag')).toBe(true);
    expect(source.includes('data-capability-health-task={capability.task}')).toBe(true);
    expect(source.includes('data-capability-health-status={status}')).toBe(true);
    expect(source.includes('data-capability-health-tooltip={capability.task}')).toBe(true);
    expect(source.includes("status === 'healthy' ? 'green' : status === 'unhealthy' ? 'red'" )).toBe(true);
  });

  test('uses the checked capability only for the optional row summary', () => {
    expect(source.includes('const aggregateCapabilityHealth = checkedCapability?.health;')).toBe(true);
    expect(source.includes('const healthStatus = aggregateCapabilityHealth?.status')).toBe(true);
    expect(source.includes('checkedCapability.task')).toBe(true);
  });
});
