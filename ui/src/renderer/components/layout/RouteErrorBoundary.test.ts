/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import { isRouteChunkLoadError } from './RouteErrorBoundary';

describe('RouteErrorBoundary chunk recovery', () => {
  test('recognizes browser and bundler dynamic import failures', () => {
    expect(
      isRouteChunkLoadError(
        new TypeError(
          'Failed to fetch dynamically imported module: https://app.example/assets/page.js'
        )
      )
    ).toBe(true);
    expect(
      isRouteChunkLoadError(
        Object.assign(new Error('Loading chunk 42 failed'), {
          name: 'ChunkLoadError',
        })
      )
    ).toBe(true);
    expect(
      isRouteChunkLoadError(new Error('Importing a module script failed'))
    ).toBe(true);
  });

  test('keeps ordinary route render failures on the in-place retry path', () => {
    expect(isRouteChunkLoadError(new Error('Cannot read properties of undefined'))).toBe(
      false
    );
  });
});
