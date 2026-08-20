/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const router = readFileSync(new URL('../../../components/layout/Router.tsx', import.meta.url), 'utf8');

describe('Creative Studio product route integration', () => {
  test('mounts every implemented product inside the focused shell', () => {
    expect(router.includes("path='canvas/:projectId'")).toBe(true);
    expect(router.includes("path='director/:projectId'")).toBe(true);
    expect(router.includes("path='image'")).toBe(true);
    expect(router.includes("path='video'")).toBe(true);
    expect(router.includes('CreativeStudioImageWorkbenchRoute')).toBe(true);
    expect(router.includes('CreativeStudioVideoWorkbenchRoute')).toBe(true);
    expect(router.includes('CreativeStudioDirectorRoute')).toBe(true);
    expect(router.includes('CreativeStudioWorkflowRoute')).toBe(true);
    expect(router.includes("path='workflows'")).toBe(true);
  });
});
