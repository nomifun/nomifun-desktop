/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const runnerSource = readFileSync(
  new URL('./RunnerPage.tsx', import.meta.url),
  'utf8'
);
const en = JSON.parse(
  readFileSync(
    new URL(
      '../../services/i18n/locales/en-US/miniApps.json',
      import.meta.url
    ),
    'utf8'
  )
);
const zh = JSON.parse(
  readFileSync(
    new URL(
      '../../services/i18n/locales/zh-CN/miniApps.json',
      import.meta.url
    ),
    'utf8'
  )
);

function sourceSection(start: string, end: string): string {
  const startAt = runnerSource.indexOf(start);
  const endAt = runnerSource.indexOf(end, startAt + start.length);
  expect(startAt).toBeGreaterThan(-1);
  expect(endAt).toBeGreaterThan(startAt);
  return runnerSource.slice(startAt, endAt);
}

function readKey(
  locale: Record<string, unknown>,
  path: string
): unknown {
  return path
    .split('.')
    .reduce<unknown>(
      (value, segment) =>
        value && typeof value === 'object'
          ? (value as Record<string, unknown>)[segment]
          : undefined,
      locale
    );
}

describe('MiniApp Runner lifecycle wiring', () => {
  test('runs Ready Service Test through the exact bridge and confirmation', () => {
    const section = sourceSection(
      'const handleTest = useCallback',
      'const handleSetServiceRunning = useCallback'
    );
    expect(section.includes('miniAppTestRequest(workshop)')).toBe(true);
    expect(section.includes('miniApps.confirm.testTitle')).toBe(true);
    expect(section.includes('ipcBridge.miniapps.test.invoke(request)')).toBe(
      true
    );
    expect(runnerSource.includes('onTest={handleTest}')).toBe(true);
    expect(runnerSource.includes('miniApps.actions.testService')).toBe(true);
  });

  test('wires all lifecycle handlers through confirmation dialogs', () => {
    const sections = [
      {
        start: 'const handleTrash = useCallback',
        end: 'const handleRestore = useCallback',
        request: 'miniAppTrashRequest(workshop)',
        bridge: 'ipcBridge.miniapps.trash.invoke(request)',
        confirm: 'miniApps.confirm.trashTitle',
        prop: 'onTrash={handleTrash}',
      },
      {
        start: 'const handleRestore = useCallback',
        end: 'const handleDelete = useCallback',
        request: 'miniAppRestoreRequest(workshop)',
        bridge: 'ipcBridge.miniapps.restore.invoke(request)',
        confirm: 'miniApps.confirm.restoreTitle',
        prop: 'onRestore={handleRestore}',
      },
      {
        start: 'const handleDelete = useCallback',
        end: 'const handleRetryDelete = useCallback',
        request: 'miniAppDeleteRequest(workshop)',
        bridge: 'ipcBridge.miniapps.delete.invoke(request)',
        confirm: 'miniApps.confirm.deleteTitle',
        prop: 'onDelete={handleDelete}',
      },
      {
        start: 'const handleRetryDelete = useCallback',
        end: 'const handleSetPublishMode = useCallback',
        request: 'miniAppRetryDeleteRequest(workshop)',
        bridge: 'ipcBridge.miniapps.retryDelete.invoke(request)',
        confirm: 'miniApps.confirm.retryDeleteTitle',
        prop: 'onRetryDelete={handleRetryDelete}',
      },
    ];

    for (const item of sections) {
      const section = sourceSection(item.start, item.end);
      expect(section.includes(item.request)).toBe(true);
      expect(section.includes('Modal.confirm({')).toBe(true);
      expect(section.includes(item.confirm)).toBe(true);
      expect(section.includes(item.bridge)).toBe(true);
      expect(runnerSource.includes(item.prop)).toBe(true);
    }
  });

  test('returns to Library after direct, retried, or reconciled deletion', () => {
    expect(runnerSource.includes('message.success(successMessage);')).toBe(
      true
    );
    expect(
      runnerSource.includes("navigate('/mini-apps', { replace: true });")
    ).toBe(true);
    expect(runnerSource.includes('isPermanentDeleteRunning(next)')).toBe(
      true
    );
    expect(runnerSource.includes('window.setTimeout(poll, 750)')).toBe(true);
    expect(
      runnerSource.includes(
        "isBackendHttpError(error) && error.status === 404"
      )
    ).toBe(true);
    expect(
      runnerSource.includes('deletionInProgressRef.current = false;')
    ).toBe(true);
  });

  test('locks normal controls outside active lifecycle and exposes retry only after failure', () => {
    expect(runnerSource.includes('const lifecycleActive =')).toBe(true);
    expect(runnerSource.includes('{lifecycleActive && (')).toBe(true);
    expect(
      runnerSource.includes(
        'disabled={controlsDisabled || buildRunning || !lifecycleActive}'
      )
    ).toBe(true);
    expect(runnerSource.includes('const permanentDeleteFailed =')).toBe(true);
    expect(
      runnerSource.includes('miniAppRetryDeleteRequest(workshop) !== null')
    ).toBe(true);
    expect(
      runnerSource.includes('miniApps.workshop.deletion.failedNotice')
    ).toBe(true);
  });

  test('ships matching English and Chinese lifecycle copy', () => {
    const keys = [
      'actions.trash',
      'actions.restore',
      'actions.deletePermanently',
      'actions.retryDelete',
      'errors.trashUnavailable',
      'errors.restoreUnavailable',
      'errors.deleteUnavailable',
      'errors.retryDeleteUnavailable',
      'confirm.trashTitle',
      'confirm.trashBody',
      'confirm.restoreTitle',
      'confirm.restoreBody',
      'confirm.deleteTitle',
      'confirm.deleteBody',
      'confirm.retryDeleteTitle',
      'confirm.retryDeleteBody',
      'workshop.deletion.trashedNotice',
      'workshop.deletion.runningNotice',
      'workshop.deletion.failedNotice',
      'workshop.operation.kindValue.permanentDelete',
      'messages.trashed',
      'messages.restored',
      'messages.deletedPermanently',
      'messages.deleteRetryCompleted',
    ];

    for (const key of keys) {
      expect(typeof readKey(en, key)).toBe('string');
      expect(typeof readKey(zh, key)).toBe('string');
    }
    expect(en.confirm.restoreBody.includes('Disabled')).toBe(true);
    expect(zh.confirm.restoreBody.includes('停用')).toBe(true);
  });
});
