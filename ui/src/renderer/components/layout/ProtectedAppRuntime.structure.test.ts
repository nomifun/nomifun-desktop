/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const readSource = (url: URL) => readFileSync(url, 'utf8');

const runtimeSource = readSource(new URL('./ProtectedAppRuntime.tsx', import.meta.url));
const routerSource = readSource(new URL('./Router.tsx', import.meta.url));

describe('protected application runtime boundary', () => {
  test('owns authentication and application-wide desktop effects without a visible layout', () => {
    expect(runtimeSource.includes('const ProtectedAppRuntime: React.FC = () =>')).toBe(true);
    expect(runtimeSource.includes('const { status } = useAuth();')).toBe(true);
    expect(runtimeSource.includes("return <Navigate to='/login' replace />;")).toBe(true);
    expect(runtimeSource.includes('<CompanionNavigateListener />')).toBe(true);
    expect(runtimeSource.includes('<CompanionWindowsSyncMount />')).toBe(true);
    expect(runtimeSource.includes('<TrayLabelsMount />')).toBe(true);
    expect(runtimeSource.includes('<Outlet />')).toBe(true);
    expect(runtimeSource.includes('layout: React.ReactElement')).toBe(false);
    expect(runtimeSource.includes('cloneElement')).toBe(false);
  });

  test('nests the unchanged workbench layout beneath the protected runtime', () => {
    expect(routerSource.includes("import ProtectedAppRuntime from '@renderer/components/layout/ProtectedAppRuntime';")).toBe(
      true
    );
    expect(/<Route element=\{<ProtectedAppRuntime \/>\}>\s*<Route element=\{layout\}>/.test(routerSource)).toBe(true);
    expect(routerSource.includes('ProtectedLayout')).toBe(false);
    expect(routerSource.includes('React.cloneElement(layout)')).toBe(false);
  });

  test('keeps both workshop routes inside the workbench layout for this refactor', () => {
    const workbenchAt = routerSource.indexOf('<Route element={layout}>');
    const workshopListAt = routerSource.indexOf("<Route path='/workshop' element={withRouteFallback(WorkshopListPage)} />");
    const workshopCanvasAt = routerSource.indexOf(
      "<Route path='/workshop/:id' element={withRouteFallback(WorkshopCanvasPage)} />"
    );
    const nestedRoutesCloseAt = routerSource.indexOf('          </Route>\n        </Route>', workshopCanvasAt);

    expect(workbenchAt).toBeGreaterThan(-1);
    expect(workshopListAt).toBeGreaterThan(workbenchAt);
    expect(workshopCanvasAt).toBeGreaterThan(workshopListAt);
    expect(nestedRoutesCloseAt).toBeGreaterThan(workshopCanvasAt);
  });
});
