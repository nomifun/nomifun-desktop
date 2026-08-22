/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { existsSync, readFileSync } from 'node:fs';

const router = readFileSync(new URL('../../../components/layout/Router.tsx', import.meta.url), 'utf8');
const retiredHomePage = new URL('./CreativeStudioHomePage.tsx', import.meta.url);

describe('Creative Studio product route integration', () => {
  test('mounts every implemented product inside the shared application layout', () => {
    const layoutAt = router.indexOf('<Route element={layout}>');
    const creativeStudioAt = router.indexOf(
      '<Route path={CREATIVE_STUDIO_ROOT_PATH} element={withRouteFallback(CreativeStudioFocusShell)}>'
    );

    expect(layoutAt).toBeGreaterThan(-1);
    expect(creativeStudioAt).toBeGreaterThan(layoutAt);
    expect(router.includes('CreativeStudioHomePage')).toBe(false);
    expect(
      router.includes('<Route index element={<Navigate to={CREATIVE_STUDIO_PROJECTS_PATH} replace />} />')
    ).toBe(true);
    expect(existsSync(retiredHomePage)).toBe(false);
    expect(router.includes("path='projects' element={withRouteFallback(CreativeStudioProjectsRoute)}")).toBe(true);
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
