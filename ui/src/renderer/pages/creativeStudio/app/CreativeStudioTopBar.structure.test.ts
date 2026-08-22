/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'node:fs';

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

  test('keeps the compact titlebar geometry on one shared centerline', () => {
    expect(
      /\.topBar\s*\{[\s\S]*?--creative-studio-top-bar-height:\s*45px;[\s\S]*?height:\s*var\(--creative-studio-top-bar-height\);[\s\S]*?min-height:\s*var\(--creative-studio-top-bar-height\);/.test(
        topBarStyles
      )
    ).toBe(true);
    expect(
      /\.inner\s*\{[\s\S]*?max-width:\s*1280px;[\s\S]*?height:\s*var\(--creative-studio-top-bar-height\);/.test(
        topBarStyles
      )
    ).toBe(true);
    expect(topBarStyles.match(/^\s*height:\s*var\(--creative-studio-top-bar-height\);/gm)?.length).toBe(6);
    expect(topBarStyles.includes('margin-left: 24px;')).toBe(true);
    expect(/\.navigation\s*\{[\s\S]*?gap:\s*20px;/.test(topBarStyles)).toBe(true);
    expect(topBarStyles.includes('padding-left: max(88px')).toBe(true);
  });

  test('optically aligns navigation icons with their labels', () => {
    expect(topBarSource.includes('className={styles.brandIcon}')).toBe(true);
    expect(topBarSource.includes('className={styles.brandLabel}')).toBe(true);
    expect(topBarSource.includes('className={styles.navigationIcon}')).toBe(true);
    expect(topBarSource.includes('className={styles.navigationLabel}')).toBe(true);
    expect(/\.brandIcon,[\s\S]*?\.navigationIcon\s*\{[\s\S]*?transform:\s*translateY\(1px\);/.test(topBarStyles)).toBe(
      true
    );
    expect(/\.brandLabel,[\s\S]*?\.navigationLabel\s*\{[\s\S]*?line-height:\s*20px;/.test(topBarStyles)).toBe(true);
  });

  test('keeps the trailing controls compact and optically aligned', () => {
    expect(topBarSource.includes('className={styles.utilityIcon}')).toBe(true);
    expect(topBarSource.includes('className={styles.backIcon}')).toBe(true);
    expect(topBarSource.includes('className={styles.backLabel}')).toBe(true);
    expect(/\.trailing\s*\{[\s\S]*?gap:\s*8px;[\s\S]*?padding-left:\s*16px;/.test(topBarStyles)).toBe(true);
    expect(/\.backButton\s*\{[\s\S]*?height:\s*32px;[\s\S]*?gap:\s*6px;[\s\S]*?padding:\s*0 10px;/.test(topBarStyles)).toBe(
      true
    );
    expect(/\.iconButton\s*\{[\s\S]*?width:\s*32px;[\s\S]*?height:\s*32px;/.test(topBarStyles)).toBe(true);
    expect(/\.backIcon\s*\{[\s\S]*?transform:\s*translateY\(1px\);/.test(topBarStyles)).toBe(true);
  });

  test('keeps source light-dark switching inside the focused product shell', () => {
    expect(topBarSource.includes('onToggleTheme')).toBe(true);
    expect(topBarSource.includes("t('settings.darkMode', { defaultValue: '深色' })")).toBe(true);
    expect(topBarSource.includes("t('settings.lightMode', { defaultValue: '浅色' })")).toBe(true);
    expect(topBarSource.includes("aria-pressed={theme === 'dark'}")).toBe(true);
    expect(topBarSource.includes("<Moon theme='outline' size={17}")).toBe(true);
    expect(topBarSource.includes("<SunOne theme='outline' size={17}")).toBe(true);
  });

  test('routes every product-shell exit through the shared product CAS leave gate', () => {
    expect(topBarSource.includes('onNavigate: (path: string) => void')).toBe(true);
    expect(topBarSource.includes('onNavigate(CREATIVE_STUDIO_ROOT_PATH)')).toBe(true);
    expect(topBarSource.includes('onNavigate(item.path)')).toBe(true);
  });

  test('keeps brand navigation and project-library navigation distinct', () => {
    expect(topBarSource.includes('to={CREATIVE_STUDIO_ROOT_PATH}')).toBe(true);
    expect(topBarSource.includes('path: CREATIVE_STUDIO_PROJECTS_PATH')).toBe(true);
    expect(homeSource.includes('data-creative-studio-home')).toBe(true);
    expect(homeSource.includes('CreativeStudioHomePage.module.css')).toBe(true);
  });
});
