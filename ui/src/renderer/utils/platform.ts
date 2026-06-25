/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

/**
 * Platform detection utilities
 * 平台检测工具函数
 */

import { getBaseUrl } from '@/common/adapter/httpBridge';

/**
 * Legacy capability gate, retained as a permanent `false`.
 *
 * Electron support has been fully removed — the app ships only as the Tauri
 * desktop shell + the WebUI browser. This used to be `Boolean(window.electronAPI)`;
 * it now always returns false. It is kept as a named flag for the few UI
 * affordances that only made sense in the old Electron shell and that Tauri
 * intentionally does NOT replicate — e.g. custom in-app window chrome (Tauri
 * uses OS-native decorations) and a couple of shell-managed OS settings.
 * For "am I in a bundled desktop shell (Tauri)?" use `isDesktopShell()`.
 *
 * TODO(cleanup): inline the remaining call sites to `false` and delete this.
 */
export const isElectronDesktop = (): boolean => false;

/**
 * Check if running inside the bundled desktop shell (Tauri) as opposed to the
 * remote WebUI browser. The Tauri shell injects `window.__backendPort` via the
 * window initialization script in apps/desktop/src/main.rs; the WebUI browser
 * build does not. Use this for shell-vs-browser runtime decisions — notably
 * auth: the desktop is single-user and must not show the login screen.
 */
export const isDesktopShell = (): boolean => {
  return typeof window !== 'undefined' && Boolean((window as { __backendPort?: number }).__backendPort);
};

/**
 * Authoritative OS string for the desktop shell, or `undefined` in the WebUI
 * browser. The Tauri shell injects `window.__os` (Rust `std::env::consts::OS`:
 * "macos" | "windows" | "linux") alongside `window.__backendPort` via the
 * window initialization script in apps/desktop/src/main.rs. This is the OS the
 * shell process actually runs on — 100% accurate, unlike `navigator.userAgent`
 * sniffing (WebView UA strings drift across OS/runtime versions).
 */
const shellOs = (): string | undefined => {
  if (typeof window === 'undefined') return undefined;
  const os = (window as { __os?: string }).__os;
  return typeof os === 'string' ? os : undefined;
};

/**
 * Check if running on macOS.
 *
 * Single source of truth for mac detection. Prefers the shell-injected
 * `window.__os` (desktop shell, authoritative); falls back to UA sniffing only
 * in the WebUI browser, where it reflects the CLIENT machine's OS — so always
 * gate desktop-shell-specific behavior on `isDesktopShell()` first.
 *
 * 检测是否运行在 macOS
 */
export const isMacOS = (): boolean => {
  const os = shellOs();
  if (os) return os === 'macos';
  return typeof navigator !== 'undefined' && /mac/i.test(navigator.userAgent);
};

/**
 * Check if running on Windows. See `isMacOS` for the detection contract.
 * 检测是否运行在 Windows
 */
export const isWindows = (): boolean => {
  const os = shellOs();
  if (os) return os === 'windows';
  return typeof navigator !== 'undefined' && /win/i.test(navigator.userAgent);
};

function isAbsoluteAssetUrl(url: string): boolean {
  return /^[a-z][a-z\d+.-]*:/i.test(url) || url.startsWith('//');
}

/**
 * Resolve a backend-served asset URL for the current environment.
 * In the desktop shell (Electron or Tauri) the renderer is NOT same-origin with
 * the backend, so backend-relative paths (e.g. `/api/assets/logos/claude.svg`)
 * must be expanded against the backend HTTP origin via `getBaseUrl()`. In the
 * WebUI browser they stay relative (same-origin reverse proxy handles them).
 *
 * Keyed on `isDesktopShell()` (the `__backendPort` signal), not
 * `isElectronDesktop()` — under Tauri the latter is false, which left every
 * backend-relative asset (agent/model logos, extension icons) pointing at the
 * dev/static server instead of the backend, so they failed to load.
 */
export const resolveBackendAssetUrl = (url: string | undefined): string | undefined => {
  if (!url) return url;
  if (isAbsoluteAssetUrl(url) || /^data:/i.test(url)) return url;
  if (url.startsWith('/')) {
    return isDesktopShell() ? `${getBaseUrl()}${url}` : url;
  }
  return url;
};

/**
 * Resolve an extension asset URL for the current environment.
 * Backend-managed extension assets are already emitted as HTTP URLs, so this
 * helper resolves app-relative backend paths into absolute backend URLs when
 * the desktop renderer is not same-origin with the backend process.
 *
 * 将扩展资源 URL 转换为当前环境可用的地址
 */
export const resolveExtensionAssetUrl = (url: string | undefined): string | undefined => {
  return resolveBackendAssetUrl(url);
};

/**
 * Open external URL in the appropriate context
 * - Electron: uses shell.openExternal via IPC (opens on local machine)
 * - WebUI: uses window.open in client browser (opens on remote client)
 *
 * 在适当的环境中打开外部链接
 * - Electron: 通过 IPC 调用 shell.openExternal（在本地机器打开）
 * - WebUI: 使用 window.open 在客户端浏览器打开（在远程客户端打开）
 */
export const openExternalUrl = async (url: string): Promise<void> => {
  if (!url) return;

  if (isDesktopShell()) {
    const { ipcBridge } = await import('@/common');
    await ipcBridge.shell.openExternal.invoke(url);
  } else {
    window.open(url, '_blank', 'noopener,noreferrer');
  }
};
