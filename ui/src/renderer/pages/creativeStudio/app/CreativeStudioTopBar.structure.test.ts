/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { existsSync, readFileSync } from 'node:fs';

const topBarSource = readFileSync(new URL('./CreativeStudioTopBar.tsx', import.meta.url), 'utf8');
const topBarStyles = readFileSync(new URL('./CreativeStudioTopBar.module.css', import.meta.url), 'utf8');
const homeSource = readFileSync(new URL('./CreativeStudioHomePage.tsx', import.meta.url), 'utf8');

describe('Creative Studio application navigation structure', () => {
  test('keeps the five source top-level destinations in source order', () => {
    const orderedRoutes = [
      'CREATIVE_STUDIO_PROJECTS_PATH',
      'CREATIVE_STUDIO_IMAGE_PATH',
      'CREATIVE_STUDIO_VIDEO_PATH',
      'CREATIVE_STUDIO_PROMPTS_PATH',
      'CREATIVE_STUDIO_ASSETS_PATH',
    ];
    const positions = orderedRoutes.map((route) => topBarSource.indexOf(`path: ${route}`));

    expect(positions.every((position) => position >= 0)).toBe(true);
    expect(positions.every((position, index) => index === 0 || position > positions[index - 1])).toBe(true);
    expect(topBarSource.includes('CREATIVE_STUDIO_AUDIO_PATH')).toBe(false);
  });

  test('retains the measured 64px source header geometry', () => {
    expect(
      /\.topBar\s*\{[\s\S]*?height:\s*64px;[\s\S]*?min-height:\s*64px;/.test(topBarStyles)
    ).toBe(true);
    expect(
      /\.inner\s*\{[\s\S]*?max-width:\s*1280px;[\s\S]*?height:\s*64px;/.test(topBarStyles)
    ).toBe(true);
    expect(topBarStyles.includes('margin-left: 32px;')).toBe(true);
    expect(topBarStyles.includes('gap: 28px;')).toBe(true);
  });

  test('uses a zero-content index instead of a fabricated landing page', () => {
    expect(homeSource.includes('const CreativeStudioHomePage: React.FC = () => null')).toBe(true);
    expect(homeSource.includes('CreativeStudioRouteRedirect')).toBe(true);
    expect(homeSource.includes('CreativeStudioHomePage.module.css')).toBe(false);
    expect(existsSync(new URL('./CreativeStudioHomePage.module.css', import.meta.url))).toBe(false);
  });
});
