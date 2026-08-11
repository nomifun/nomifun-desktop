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

  test('the entry sits in the 常用 group directly after the workshop', () => {
    const workshopAt = siderSource.indexOf('<SiderWorkshopEntry');
    const miniAppsAt = siderSource.indexOf('<SiderMiniAppsEntry');
    const dataSectionAt = siderSource.indexOf("t('common.siderSection.data')");
    expect(workshopAt).toBeGreaterThan(-1);
    expect(miniAppsAt).toBeGreaterThan(workshopAt);
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

  test('both mini-app routes are registered behind the route fallback', () => {
    expect(routerSource.includes("path='/mini-apps' element={withRouteFallback(MiniAppsListPage)}")).toBe(true);
    expect(routerSource.includes("path='/mini-apps/:id' element={withRouteFallback(MiniAppRunnerPage)}")).toBe(true);
    expect(routerSource.includes("import('@renderer/pages/miniApps')")).toBe(true);
    expect(routerSource.includes("import('@renderer/pages/miniApps/RunnerPage')")).toBe(true);
  });
});
