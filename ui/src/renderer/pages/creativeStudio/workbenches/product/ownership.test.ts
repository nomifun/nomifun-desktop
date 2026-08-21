/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  parseStandaloneProjectQuery,
  standaloneWorkbenchOwner,
  STANDALONE_VIDEO_MAX_CONCURRENT_TASKS,
} from './ownership';

const PROJECT_ID = '0190f5fe-7c00-7a00-8000-000000000701';

describe('standalone workbench ownership', () => {
  test('requires one explicit canonical query and never infers a project', () => {
    expect(parseStandaloneProjectQuery('').state).toBe('missing');
    expect(parseStandaloneProjectQuery('?projectId=recent').state).toBe('invalid');
    expect(
      parseStandaloneProjectQuery(`?projectId=${PROJECT_ID}&projectId=${PROJECT_ID}`).state
    ).toBe('invalid');
    expect(parseStandaloneProjectQuery(`?projectId=${PROJECT_ID}`)).toEqual({
      state: 'valid',
      projectId: PROJECT_ID,
    });
  });

  test('creates only the canonical standalone owner union branch', () => {
    expect(standaloneWorkbenchOwner(PROJECT_ID, 'image')).toEqual({
      kind: 'standalone_workbench',
      projectId: PROJECT_ID,
      workbenchKind: 'image',
    });
    let error: unknown = null;
    try {
      standaloneWorkbenchOwner('recent', 'video');
    } catch (reason) {
      error = reason;
    }
    expect(error instanceof Error).toBe(true);
  });

  test('keeps video fan-out closed until its dedicated product gate', () => {
    expect(STANDALONE_VIDEO_MAX_CONCURRENT_TASKS).toBe(1);
  });
});
