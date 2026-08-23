/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./CreativeStudioSider.tsx', import.meta.url), 'utf8');
const settingsSource = readFileSync(
  new URL('../../settings/components/SettingsSider.tsx', import.meta.url),
  'utf8'
);

describe('Creative Studio sidebar navigation', () => {
  test('keeps the product destinations in sidebar order without a duplicate home entry', () => {
    const orderedRoutes = [
      'path: safeCanvasesResumePath',
      'path: CREATIVE_STUDIO_IMAGE_PATH',
      'path: CREATIVE_STUDIO_VIDEO_PATH',
      'path: CREATIVE_STUDIO_PROMPTS_PATH',
      'path: CREATIVE_STUDIO_ASSETS_PATH',
    ];
    const positions = orderedRoutes.map((route) => source.indexOf(route));

    expect(positions.every((position) => position >= 0)).toBe(true);
    expect(positions.every((position, index) => index === 0 || position > positions[index - 1])).toBe(true);
    expect(source.includes('CREATIVE_STUDIO_ROOT_PATH')).toBe(false);
    expect(source.includes("section: 'home'")).toBe(false);
    expect(source.includes('<FolderOpen')).toBe(false);
    expect(source.includes('CREATIVE_STUDIO_AUDIO_PATH')).toBe(false);
  });

  test('reuses the Settings rail item and collapsed-state contracts', () => {
    for (const marker of [
      'settings-sider',
      'settings-sider--collapsed',
      'settings-sider__item',
      'settings-sider__item-label',
    ]) {
      expect(source.includes(marker)).toBe(true);
      expect(settingsSource.includes(marker)).toBe(true);
    }
    expect(source.includes('data-creative-studio-sider')).toBe(true);
    expect(source.includes("? 'canvases' : section")).toBe(true);
    expect(
      source.includes(
        'canvasesResumePath = CREATIVE_STUDIO_CANVASES_PATH'
      )
    ).toBe(true);
    expect(
      source.includes(
        'normalizeCreativeStudioCanvasesResumeLocation(canvasesResumePath)'
      )
    ).toBe(true);
    expect(source.includes('path: safeCanvasesResumePath')).toBe(true);
    expect(source.includes("section: 'projects'")).toBe(false);
    expect(source.includes('CREATIVE_STUDIO_PROJECTS_PATH')).toBe(false);
    expect(
      source.includes("t('creativeStudio.navigation.canvases')")
    ).toBe(true);
    expect(source.includes('creativeStudio.navigation.projects')).toBe(false);
  });

  test('contains no product-specific titlebar or window controls', () => {
    expect(source.includes('WindowControls')).toBe(false);
    expect(source.includes('data-tauri-drag-region')).toBe(false);
    expect(source.includes('toggleMaximize')).toBe(false);
  });
});
