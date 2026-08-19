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
const themeRuntimeSource = readSource(new URL('./AppThemeRuntime.tsx', import.meta.url));
const workbenchLayoutSource = readSource(new URL('./Layout.tsx', import.meta.url));
const focusShellSource = readSource(
  new URL('../../pages/creativeStudio/app/CreativeStudioFocusShell.tsx', import.meta.url)
);
const focusTopBarSource = readSource(
  new URL('../../pages/creativeStudio/app/CreativeStudioTopBar.tsx', import.meta.url)
);

describe('protected application runtime boundary', () => {
  test('owns authentication and application-wide desktop effects without a visible layout', () => {
    expect(runtimeSource.includes('const ProtectedAppRuntime: React.FC = () =>')).toBe(true);
    expect(runtimeSource.includes('const { status } = useAuth();')).toBe(true);
    expect(runtimeSource.includes("return <Navigate to='/login' replace />;")).toBe(true);
    expect(runtimeSource.includes('<CompanionNavigateListener />')).toBe(true);
    expect(runtimeSource.includes('<CompanionWindowsSyncMount />')).toBe(true);
    expect(runtimeSource.includes('<TrayLabelsMount />')).toBe(true);
    expect(runtimeSource.includes('<ProtectedNavigationRuntime />')).toBe(true);
    expect(runtimeSource.includes('useDeepLink();')).toBe(true);
    expect(runtimeSource.includes('useNotificationClick();')).toBe(true);
    expect(runtimeSource.includes('<Outlet />')).toBe(true);
    expect(runtimeSource.includes('layout: React.ReactElement')).toBe(false);
    expect(runtimeSource.includes('cloneElement')).toBe(false);
  });

  test('keeps the workbench layout beneath the protected runtime', () => {
    expect(routerSource.includes("import ProtectedAppRuntime from '@renderer/components/layout/ProtectedAppRuntime';")).toBe(
      true
    );
    const protectedRuntimeAt = routerSource.indexOf('<Route element={<ProtectedAppRuntime />}>');
    const workbenchAt = routerSource.indexOf('<Route element={layout}>');
    expect(protectedRuntimeAt).toBeGreaterThan(-1);
    expect(workbenchAt).toBeGreaterThan(protectedRuntimeAt);
    expect(routerSource.includes('ProtectedLayout')).toBe(false);
    expect(routerSource.includes('React.cloneElement(layout)')).toBe(false);
  });

  test('keeps the application theme alive above every protected routed layout', () => {
    const themeRuntimeAt = runtimeSource.indexOf('<AppThemeRuntime />');
    const routedLayoutAt = runtimeSource.indexOf('<Outlet />');

    expect(themeRuntimeAt).toBeGreaterThan(-1);
    expect(routedLayoutAt).toBeGreaterThan(themeRuntimeAt);
    expect(themeRuntimeSource.includes("const CUSTOM_CSS_STYLE_ID = 'user-defined-custom-css';")).toBe(true);
    expect(themeRuntimeSource.includes('broadcastCustomCssSync(customCss);')).toBe(true);
    expect(themeRuntimeSource.includes('ensureThemeControlContract();')).toBe(true);
    expect(themeRuntimeSource.includes('observer.observe(document.head, { childList: true });')).toBe(true);
  });

  test('leaves the visible workbench layout free of application theme ownership', () => {
    expect(workbenchLayoutSource.includes('loadAndHealCustomCss')).toBe(false);
    expect(workbenchLayoutSource.includes('user-defined-custom-css')).toBe(false);
    expect(workbenchLayoutSource.includes('broadcastCustomCssSync')).toBe(false);
    expect(workbenchLayoutSource.includes('THEME_CONTROL_CONTRACT_STYLE_ID')).toBe(false);
    expect(workbenchLayoutSource.includes('useDeepLink')).toBe(false);
    expect(workbenchLayoutSource.includes('useNotificationClick')).toBe(false);
  });

  test('routes the rebuilt workshop through a focused shell beside the workbench layout', () => {
    const protectedRuntimeAt = routerSource.indexOf('<Route element={<ProtectedAppRuntime />}>');
    const focusRouteAt = routerSource.indexOf(
      '<Route path={CREATIVE_STUDIO_ROOT_PATH} element={withRouteFallback(CreativeStudioFocusShell)}>'
    );
    const workbenchAt = routerSource.indexOf('<Route element={layout}>');

    expect(protectedRuntimeAt).toBeGreaterThan(-1);
    expect(focusRouteAt).toBeGreaterThan(protectedRuntimeAt);
    expect(workbenchAt).toBeGreaterThan(focusRouteAt);
    expect(routerSource.includes("import('@renderer/pages/workshop')")).toBe(false);
    expect(routerSource.includes("import('@renderer/pages/workshop/CanvasPage')")).toBe(false);
  });

  test('keeps the focus shell free of workbench-only chrome and exposes a stable return path', () => {
    expect(focusShellSource.includes('void navigate(WORKBENCH_HOME_PATH, { replace: true });')).toBe(true);
    expect(focusShellSource.includes('data-creative-studio-focus-shell')).toBe(true);
    expect(focusShellSource.includes("classNames('creative-studio-root', styles.shell)")).toBe(true);
    expect(focusShellSource.includes("id='creative-studio-portal-root'")).toBe(true);
    expect(focusShellSource.includes('<CreativeStudioTopBar')).toBe(true);
    expect(focusShellSource.includes('<Outlet />')).toBe(true);
    expect(focusShellSource.includes('PwaPullToRefresh')).toBe(false);
    expect(focusShellSource.includes('<Sider')).toBe(false);
    expect(focusShellSource.includes('LayoutContext')).toBe(false);
  });

  test('gives the focused product independent draggable window chrome', () => {
    expect(focusTopBarSource.includes('data-creative-studio-top-bar')).toBe(true);
    expect(focusTopBarSource.includes('data-tauri-drag-region')).toBe(true);
    expect(focusTopBarSource.includes('ipcBridge.windowControls.toggleMaximize.invoke()')).toBe(true);
    expect(focusTopBarSource.includes('{showWindowControls && <WindowControls />}')).toBe(true);
    expect(focusTopBarSource.includes('useLayoutContext')).toBe(false);
    expect(focusTopBarSource.includes('useNavigationHistory')).toBe(false);
  });

});
