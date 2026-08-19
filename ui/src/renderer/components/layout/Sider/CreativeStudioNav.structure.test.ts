/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { existsSync, readFileSync } from 'node:fs';

const readSource = (url: URL) => readFileSync(url, 'utf8');
const siderSource = readSource(new URL('./index.tsx', import.meta.url));
const navBarrelSource = readSource(new URL('./SiderNav/index.ts', import.meta.url));
const entrySource = readSource(new URL('./SiderNav/SiderCreativeStudioEntry.tsx', import.meta.url));
const routesSource = readSource(new URL('../../../pages/creativeStudio/app/routes.ts', import.meta.url));
const legacyEntryUrl = new URL('./SiderNav/SiderWorkshopEntry.tsx', import.meta.url);

describe('Creative Studio primary navigation', () => {
  test('owns the former workshop slot with one canonical product route', () => {
    expect(routesSource.includes("CREATIVE_STUDIO_ROOT_PATH = '/workshop'")).toBe(true);
    expect(siderSource.includes('navTo(CREATIVE_STUDIO_ROOT_PATH)')).toBe(true);
    expect(siderSource.includes('pathname.startsWith(CREATIVE_STUDIO_ROOT_PATH)')).toBe(true);
    expect(siderSource.includes('<SiderCreativeStudioEntry')).toBe(true);
  });

  test('removes the beta workshop entry instead of keeping parallel navigation', () => {
    expect(
      navBarrelSource.includes(
        "export { default as SiderCreativeStudioEntry } from './SiderCreativeStudioEntry';"
      )
    ).toBe(true);
    expect(navBarrelSource.includes('SiderWorkshopEntry')).toBe(false);
    expect(entrySource.includes("t('creativeStudio.title')")).toBe(true);
    expect(entrySource.includes('workshop.beta')).toBe(false);
    expect(entrySource.includes('Beta')).toBe(false);
    expect(existsSync(legacyEntryUrl)).toBe(false);
  });
});
