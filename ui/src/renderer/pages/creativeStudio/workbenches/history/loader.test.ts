/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { testUuid } from '../../canvas/core/testFixtures';
import type { CreativeTask } from '../../tasks';
import {
  appendStandaloneHistoryPage,
  combineStandaloneHistoryTasks,
  loadStandaloneWorkbenchHistoryBootstrap,
  type StandaloneTaskHistoryReader,
} from './loader';

const task = (
  index: number,
  submittedAt: number,
  status: CreativeTask['status']
): CreativeTask => ({
  taskId: testUuid(index),
  owner: { kind: 'standalone_workbench', workbenchKind: 'image' },
  providerId: testUuid(850),
  model: 'image-v1',
  task: 'image_generation',
  capability: 't2i',
  parameters: { prompt: `prompt-${index}`, interface_mode: 'images', quality: 'auto', aspect: '1:1', count: 1, width: 1024, height: 1024 },
  inputs: [],
  status,
  error: status === 'failed' ? { kind: 'provider', message: 'failed', httpStatus: 500 } : null,
  resultAssetIds: [],
  attempt: 1,
  submittedAt,
  startedAt: status === 'queued' ? null : submittedAt + 1,
  finishedAt: status === 'failed' ? submittedAt + 2 : null,
  deletedAt: null,
});

describe('standalone history loader', () => {
  test('loads visible history and pages the complete active recovery inventory', async () => {
    const visibleActive = task(802, 300, 'running');
    const visibleOnlyActive = task(806, 250, 'queued');
    const terminal = task(803, 200, 'failed');
    const olderActive = task(804, 100, 'queued');
    const reader: StandaloneTaskHistoryReader = {
      listStandalone: async (query) => {
        if (!query.activeOnly) {
          return {
            items: [visibleActive, visibleOnlyActive, terminal],
            nextCursor: `200:${terminal.taskId}`,
          };
        }
        if (!query.cursor) {
          return {
            items: [{ ...visibleActive, status: 'queued', startedAt: null }, olderActive],
            nextCursor: `100:${olderActive.taskId}`,
          };
        }
        return { items: [], nextCursor: null };
      },
    };
    const result = await loadStandaloneWorkbenchHistoryBootstrap(reader, {
      workbenchKind: 'image',
    });
    expect(result.tasks).toEqual([visibleActive, visibleOnlyActive, terminal]);
    expect(result.activeTasks).toEqual([visibleActive, visibleOnlyActive, olderActive]);
    expect(result.nextCursor).toBe(`200:${terminal.taskId}`);
    expect(combineStandaloneHistoryTasks(result.tasks, result.activeTasks)).toEqual([
      visibleActive,
      visibleOnlyActive,
      terminal,
      olderActive,
    ]);
  });

  test('rejects a repeated task identity across appended history pages', () => {
    const first = task(805, 20, 'failed');
    let error: unknown = null;
    try {
      appendStandaloneHistoryPage([first], { items: [{ ...first }], nextCursor: null });
    } catch (reason) {
      error = reason;
    }
    expect(error instanceof Error).toBe(true);
  });
});
