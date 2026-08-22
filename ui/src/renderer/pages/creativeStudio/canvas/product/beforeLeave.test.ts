/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from 'bun:test';

import {
  hasCreativeCanvasProductBeforeLeave,
  registerCreativeCanvasProductBeforeLeave,
  requestCreativeCanvasProductBeforeLeave,
} from './beforeLeave';

let cleanup: (() => void) | null = null;

afterEach(() => {
  cleanup?.();
  cleanup = null;
});

describe('Creative Canvas product before-leave registry', () => {
  test('defaults to safe when no canvas route is mounted', async () => {
    expect(hasCreativeCanvasProductBeforeLeave()).toBe(false);
    expect(await requestCreativeCanvasProductBeforeLeave()).toBe(true);
  });

  test('exposes the active async CAS decision and unregisters by ownership', async () => {
    cleanup = registerCreativeCanvasProductBeforeLeave(async () => false);
    expect(hasCreativeCanvasProductBeforeLeave()).toBe(true);
    expect(await requestCreativeCanvasProductBeforeLeave()).toBe(false);

    const firstCleanup = cleanup;
    cleanup = registerCreativeCanvasProductBeforeLeave(async () => true);
    firstCleanup();
    expect(await requestCreativeCanvasProductBeforeLeave()).toBe(true);

    cleanup();
    cleanup = null;
    expect(hasCreativeCanvasProductBeforeLeave()).toBe(false);
  });

  test('fails closed if the registered callback throws', async () => {
    cleanup = registerCreativeCanvasProductBeforeLeave(async () => {
      throw new Error('flush failed');
    });
    expect(await requestCreativeCanvasProductBeforeLeave()).toBe(false);
  });
});
