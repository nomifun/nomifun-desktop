/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

const siderSource = readSource(new URL('./index.tsx', import.meta.url));
const navBarrelSource = readSource(new URL('./SiderNav/index.ts', import.meta.url));
const entrySource = readSource(new URL('./SiderNav/SiderMiniAppsEntry.tsx', import.meta.url));
const routerSource = readSource(new URL('../Router.tsx', import.meta.url));

describe('mini-apps rail navigation', () => {
  test('the rail carries a mini-apps entry that routes through navTo', () => {
    expect(navBarrelSource.includes("export { default as SiderMiniAppsEntry } from './SiderMiniAppsEntry';")).toBe(true);
    expect(siderSource.includes('SiderMiniAppsEntry')).toBe(true);
    // navTo (not a bare navigate) is what closes the mobile drawer and clears
    // the rail tooltips.
    expect(siderSource.includes("navTo('/mini-apps')")).toBe(true);
    expect(siderSource.includes("pathname.startsWith('/mini-apps')")).toBe(true);
  });

  test('the entry stays active for the runner route, not just the library', () => {
    // `pathname === '/mini-apps'` would blank the rail highlight the moment a
    // mini-app is opened at /mini-apps/:id.
    expect(siderSource.includes("isActive={pathname === '/mini-apps'}")).toBe(false);
  });

  test('the entry sits in the 常用 group directly after Creative Studio', () => {
    const creativeStudioAt = siderSource.indexOf('<SiderCreativeStudioEntry');
    const miniAppsAt = siderSource.indexOf('<SiderMiniAppsEntry');
    const dataSectionAt = siderSource.indexOf("t('common.siderSection.data')");
    expect(creativeStudioAt).toBeGreaterThan(-1);
    expect(miniAppsAt).toBeGreaterThan(creativeStudioAt);
    expect(miniAppsAt).toBeLessThan(dataSectionAt);
  });

  test('the entry labels itself from the miniApps namespace with a plain icon import', () => {
    expect(entrySource.includes("t('miniApps.nav.entry')")).toBe(true);
    expect(entrySource.includes("import { ApplicationOne } from '@icon-park/react';")).toBe(true);
    // An aliased icon import survives tsc but the build-time icon rewrite turns
    // it into illegal syntax, so the module 500s at runtime.
    const iconImportLine = entrySource.split('\n').find((line) => line.includes('@icon-park/react')) ?? '';
    expect(iconImportLine.includes(' as ')).toBe(false);
    expect(iconImportLine.includes('* ')).toBe(false);
  });

  test('library, runner, and creator routes use their product pages behind the route fallback', () => {
    const source = routerSource.replace(/\s+/g, ' ');
    const routes = [
      ['/mini-apps', 'MiniAppsListPage'],
      ['/mini-apps/new', 'MiniAppCreatorPage'],
      ['/mini-apps/create/:draftId', 'MiniAppCreatorPage'],
      ['/mini-apps/:id', 'MiniAppRunnerPage'],
    ];
    for (const [path, component] of routes) {
      expect(source).toContain(
        `<Route path='${path}' element={withRouteFallback(${component})} />`
      );
    }

    // Check the route component's lazy binding, not an unrelated import or
    // the compatibility re-export left at the former page entry point.
    const pages = [
      ['MiniAppsListPage', 'MiniAppLibraryPage'],
      ['MiniAppRunnerPage', 'MiniAppRunPage'],
      ['MiniAppCreatorPage', 'MiniAppCreatorPage'],
    ];
    for (const [component, page] of pages) {
      expect(source).toContain(
        `const ${component} = React.lazy(() => import('@renderer/pages/miniApps/${page}'));`
      );
    }
  });
});
