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
const creativeSiderSource = readSource(
  new URL('../../../pages/creativeStudio/app/CreativeStudioSider.tsx', import.meta.url)
);
const routesSource = readSource(new URL('../../../pages/creativeStudio/app/routes.ts', import.meta.url));
const routerSource = readSource(new URL('../Router.tsx', import.meta.url));
const legacyEntryUrl = new URL('./SiderNav/SiderWorkshopEntry.tsx', import.meta.url);
const legacyAssetPageUrl = new URL('../../../pages/assets/index.tsx', import.meta.url);

describe('Creative Studio primary navigation', () => {
  test('owns the former workshop slot with one canonical product route', () => {
    expect(routesSource.includes("CREATIVE_STUDIO_ROOT_PATH = '/workshop'")).toBe(true);
    expect(siderSource.includes('navTo(CREATIVE_STUDIO_PROJECTS_PATH)')).toBe(true);
    expect(siderSource.includes('isCreativeStudioPath(pathname)')).toBe(true);
    expect(siderSource.includes('<SiderCreativeStudioEntry')).toBe(true);
    expect(siderSource.includes('<CreativeStudioSider')).toBe(true);
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

  test('routes the workbench asset entry into the canonical product', () => {
    expect(routesSource.includes("CREATIVE_STUDIO_ASSETS_PATH = '/workshop/assets'")).toBe(true);
    expect(siderSource.includes('navTo(CREATIVE_STUDIO_ASSETS_PATH)')).toBe(true);
    expect(siderSource.includes("navTo('/assets')")).toBe(false);
    expect(routerSource.includes("import('@renderer/pages/assets')")).toBe(false);
    expect(routerSource.includes("path='/assets'")).toBe(false);
    expect(existsSync(legacyAssetPageUrl)).toBe(false);
  });

  test('switches to Settings-style product navigation with the workbench return pinned below it', () => {
    expect(creativeSiderSource.includes('settings-sider__item')).toBe(true);
    expect(siderSource.includes('isCreativeStudio ? (')).toBe(true);
    expect(siderSource.includes("backLabel={t('creativeStudio.focus.backToWorkbench')}")).toBe(true);
    expect(siderSource.includes('onSettingsClick={handleReturnToWorkbench}')).toBe(true);
    expect(siderSource.includes('handleCreativeStudioNavigation(WORKBENCH_HOME_PATH, true)')).toBe(true);
    expect(siderSource.includes('requestCreativeStudioBeforeLeave')).toBe(true);
  });
});
