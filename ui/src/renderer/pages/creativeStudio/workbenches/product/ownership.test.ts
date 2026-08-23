/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  standaloneWorkbenchOwner,
  STANDALONE_VIDEO_MAX_CONCURRENT_TASKS,
} from './ownership';

describe('standalone workbench ownership', () => {
  test('creates an installation-owned owner without a Canvas identity', () => {
    expect(standaloneWorkbenchOwner('image')).toEqual({
      kind: 'standalone_workbench',
      workbenchKind: 'image',
    });
    expect(standaloneWorkbenchOwner('video')).toEqual({
      kind: 'standalone_workbench',
      workbenchKind: 'video',
    });
  });

  test('keeps video fan-out closed until its dedicated product gate', () => {
    expect(STANDALONE_VIDEO_MAX_CONCURRENT_TASKS).toBe(1);
  });
});
