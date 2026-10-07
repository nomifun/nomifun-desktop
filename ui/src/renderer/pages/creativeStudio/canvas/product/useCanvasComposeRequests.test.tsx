/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import { useCanvasComposeRequests } from './useCanvasComposeRequests';

afterEach(cleanup);

test('submission admission and retry state are independent for each canvas node', () => {
  const { result } = renderHook(() => useCanvasComposeRequests<{ nodeId: string; taskId: string }>('canvas'));
  act(() => {
    result.current.setBusy(true, 'first');
    result.current.setBusy(true, 'second');
    result.current.setIssue({ nodeId: 'first', message: 'Uncertain first POST' });
    result.current.setSubmission({ nodeId: 'first', taskId: 'first-task' });
    result.current.setIssue({ nodeId: 'second', message: 'Uncertain second POST' });
    result.current.setSubmission({ nodeId: 'second', taskId: 'second-task' });
  });
  act(() => {
    result.current.setBusy(false, 'first');
    result.current.setIssue(null, 'first');
    result.current.setSubmission(null, 'first');
  });
  expect(result.current.busy).toBe(true);
  expect(result.current.isBusy('first')).toBe(false);
  expect(result.current.isBusy('second')).toBe(true);
  expect(result.current.issues.get('second')?.message).toBe('Uncertain second POST');
  expect(result.current.submissions.get('second')?.taskId).toBe('second-task');
});

test('late callbacks from an old canvas cannot change the next canvas request state', () => {
  const { result, rerender } = renderHook(({ scope }) => useCanvasComposeRequests<{ nodeId: string }>(scope), {
    initialProps: { scope: 'old-canvas' },
  });
  const old = result.current;
  act(() => old.setBusy(true, 'node'));
  rerender({ scope: 'new-canvas' });
  act(() => {
    result.current.setBusy(true, 'node');
    old.setBusy(false, 'node');
    old.setIssue({ nodeId: 'node', message: 'Old canvas error' });
    old.setSubmission({ nodeId: 'node' });
  });
  expect(result.current.isBusy('node')).toBe(true);
  expect(result.current.issues.size).toBe(0);
  expect(result.current.submissions.size).toBe(0);
});
