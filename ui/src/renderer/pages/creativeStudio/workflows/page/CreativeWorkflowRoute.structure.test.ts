/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const route = readFileSync(new URL('./CreativeWorkflowRoute.tsx', import.meta.url), 'utf8');
const page = readFileSync(
  new URL('./CreativeWorkflowWorkspacePage.tsx', import.meta.url),
  'utf8'
);

describe('Creative Workflow route composition', () => {
  test('uses the canonical repository and does not revive source local persistence', () => {
    expect(page.includes('creativeWorkflowRepository')).toBe(true);
    expect(page.includes('localStorage')).toBe(false);
    expect(page.includes('localforage')).toBe(false);
    expect(page.includes('/api/v1/workflows')).toBe(false);
    expect(route.includes("navigate('/models')")).toBe(true);
    expect(route.includes('useCreativeWorkflowRuntime')).toBe(true);
    expect(route.includes('creativeAssetClient')).toBe(true);
  });
});
