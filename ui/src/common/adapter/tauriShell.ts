/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Tauri desktop-shell adapter — the Tauri-native replacement for the former
 * Electron `bridge.buildProvider/buildEmitter` IPC channels that ipcBridge.ts
 * used for OS-shell operations.
 *
 * Every operation is implemented with a Tauri v2 JS API (a plugin or
 * `@tauri-apps/api`) and is GUARDED by `isTauriRuntime()`:
 *   - In the Tauri desktop shell → the real Tauri call runs.
 *   - In the WebUI browser       → providers return a web-safe fallback and
 *                                  emitters are inert (no transport, no throw).
 *
 * Operations with no Tauri equivalent (Chrome DevTools Protocol, GPU-process
 * recovery, devtools open/close, renderer-log piping, WebUI-server lifecycle,
 * close-to-tray window behavior) are intentionally DEGRADED to safe stubs here
 * — marked `DEGRADE_STUB`. They no longer depend on the deleted `@/platform`
 * bridge. Each carries a TODO if a real Tauri port is wanted later.
 *
 * Tauri modules are loaded via dynamic `import()` inside the guarded branch so
 * the WebUI browser bundle never evaluates Tauri IPC code.
 */
import { isTauriRuntime } from './tauriRuntime';
import type { UpdaterInstallContext } from './tauriUpdateInstall';

// ---------------------------------------------------------------------------
// Channel shapes — mirror the bridge.buildProvider / bridge.buildEmitter API
// that existing ipcBridge consumers depend on.
// ---------------------------------------------------------------------------

export interface ShellProvider<Data, Params> {
  /** No-op on the renderer side; kept for API compatibility with bridge.buildProvider. */
  provider: () => void;
  invoke: (params: Params) => Promise<Data>;
}

export interface ShellEmitter<Params> {
  on: (callback: (params: Params) => void) => () => void;
  emit: (params: Params) => void;
}

/** A provider backed by a Tauri call, with a web-safe fallback for the browser. */
export function shellProvider<Data, Params = void>(
  handler: (params: Params) => Promise<Data>,
  webFallback: Data | ((params: Params) => Data | Promise<Data>)
): ShellProvider<Data, Params> {
  return {
    provider: () => {},
    invoke: async (params: Params): Promise<Data> => {
      if (isTauriRuntime()) return handler(params);
      return typeof webFallback === 'function'
        ? (webFallback as (p: Params) => Data | Promise<Data>)(params)
        : webFallback;
    },
  };
}

/** DEGRADE_STUB provider: returns a constant value in every runtime (no Tauri equivalent). */
export function stubShellProvider<Data, Params = void>(value: Data | (() => Data)): ShellProvider<Data, Params> {
  return {
    provider: () => {},
    invoke: async (): Promise<Data> => (typeof value === 'function' ? (value as () => Data)() : value),
  };
}

/** An emitter backed by a Tauri event subscription, inert in the browser. */
export function shellEmitter<Params = void>(
  subscribe: (callback: (params: Params) => void) => Promise<() => void>
): ShellEmitter<Params> {
  return {
    on: (callback: (params: Params) => void): (() => void) => {
      if (!isTauriRuntime()) return () => {};
      let unlisten: (() => void) | null = null;
      let disposed = false;
      void subscribe(callback)
        .then((un) => {
          if (disposed) un();
          else unlisten = un;
        })
        .catch(() => {});
      return () => {
        disposed = true;
        if (unlisten) unlisten();
      };
    },
    emit: () => {},
  };
}

/** DEGRADE_STUB emitter: never fires (no Tauri source for this signal). */
export function noopEmitter<Params = void>(): ShellEmitter<Params> {
  return {
    on: () => () => {},
    emit: () => {},
  };
}

// ---------------------------------------------------------------------------
// Operations (Tauri v2 JS APIs)
// ---------------------------------------------------------------------------

/** Restart the desktop shell (tauri-plugin-process). */
export async function tauriRelaunch(): Promise<void> {
  const { relaunch } = await import('@tauri-apps/plugin-process');
  await relaunch();
}

/** OS directory paths (@tauri-apps/api/path). */
export async function tauriGetPath(name: 'desktop' | 'home' | 'downloads'): Promise<string> {
  const path = await import('@tauri-apps/api/path');
  if (name === 'home') return path.homeDir();
  if (name === 'downloads') return path.downloadDir();
  return path.desktopDir();
}

// Tauri exposes no zoom *getter*; remember the last value set this session.
let lastZoomFactor = 1;
export async function tauriSetZoom(factor: number): Promise<number> {
  const { getCurrentWebview } = await import('@tauri-apps/api/webview');
  await getCurrentWebview().setZoom(factor);
  lastZoomFactor = factor;
  return factor;
}
export function tauriGetZoom(): number {
  return lastZoomFactor;
}

/** 开/关 OS 级保持唤醒(防系统休眠),走桌面 Tauri command;非桌面环境会抛错,由上层吞掉。
 *  Apply/clear the OS-level keep-awake (sleep inhibitor) via the desktop command. */
export async function tauriSetKeepAwake(enabled: boolean): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('set_keep_awake', { enabled });
}

/** 本地化原生系统托盘菜单(「显示」「退出」)。Rust 侧无法解析 i18n,创建时用英文兜底,
 *  渲染层在挂载/切换语言时调用此命令传入译文。非桌面环境会抛错,由上层吞掉。
 *  Localize the native system-tray menu labels (Show / Quit) via the desktop command. */
export async function tauriSetTrayLabels(show: string, quit: string): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('set_tray_labels', { show, quit });
}

/** Inspect whether the running desktop bundle can be safely replaced by the updater. */
export async function tauriGetUpdaterInstallContext(): Promise<UpdaterInstallContext> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<UpdaterInstallContext>('get_updater_install_context');
}

export type TauriDownloadUpdatePhase = 'checking' | 'retrying' | 'downloading' | 'downloaded';

export interface TauriDownloadUpdateProgress {
  phase: TauriDownloadUpdatePhase;
  chunkLength?: number;
  contentLength?: number;
}

/** Download and retain a signature-verified update in the Rust-owned cache. */
export async function tauriDownloadUpdate(
  version: string,
  onProgress: (event: TauriDownloadUpdateProgress) => void
): Promise<void> {
  const { Channel, invoke } = await import('@tauri-apps/api/core');
  const onEvent = new Channel<TauriDownloadUpdateProgress>(onProgress);
  await invoke('download_update', { version, onEvent });
}

/** Install the already-downloaded update through the Rust-owned fail-closed path. */
export async function tauriInstallUpdate(version: string): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('install_update', { version });
}

export type TauriUpdatePackageState = 'empty' | 'downloading' | 'ready' | 'installing';

export interface TauriUpdatePackageStatus {
  state: TauriUpdatePackageState;
  /** The version the active state refers to; only `ready` means installable. */
  version: string | null;
}

/**
 * The authoritative native answer to "is an update package installable right
 * now". The renderer must not keep its own copy of this fact: a module-global
 * mirror drifted out of sync with the Rust slot and silently disabled the
 * install action while a perfectly good verified package was still retained.
 */
export async function tauriUpdatePackageStatus(): Promise<TauriUpdatePackageStatus> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<TauriUpdatePackageStatus>('update_package_status');
}

/** Electron-style OpenDialog options accepted by call sites. */
export interface ShellOpenDialogOptions {
  properties?: Array<'openFile' | 'openDirectory' | 'multiSelections' | 'createDirectory' | 'showHiddenFiles'>;
  filters?: Array<{ name: string; extensions: string[] }>;
  defaultPath?: string;
}

/** Native open file/folder dialog (tauri-plugin-dialog), normalized to string[] | undefined. */
export async function tauriOpenDialog(options?: ShellOpenDialogOptions): Promise<string[] | undefined> {
  const { open } = await import('@tauri-apps/plugin-dialog');
  const props = options?.properties ?? [];
  const result = await open({
    directory: props.includes('openDirectory'),
    multiple: props.includes('multiSelections'),
    defaultPath: options?.defaultPath,
    filters: options?.filters,
  });
  if (result == null) return undefined;
  return Array.isArray(result) ? result : [result];
}

/** OS auto-launch (tauri-plugin-autostart). */
export async function tauriIsAutostartEnabled(): Promise<boolean> {
  const { isEnabled } = await import('@tauri-apps/plugin-autostart');
  return isEnabled();
}
export async function tauriSetAutostart(enabled: boolean): Promise<void> {
  const mod = await import('@tauri-apps/plugin-autostart');
  if (enabled) await mod.enable();
  else await mod.disable();
}

/** Native OS notification (tauri-plugin-notification). */
export async function tauriSendNotification(opts: { title: string; body: string; icon?: string }): Promise<void> {
  const mod = await import('@tauri-apps/plugin-notification');
  let granted = await mod.isPermissionGranted();
  if (!granted) granted = (await mod.requestPermission()) === 'granted';
  if (granted) mod.sendNotification({ title: opts.title, body: opts.body, icon: opts.icon });
}

const ADD_PROVIDER_DEEP_LINK_PARAMS = new Set(['base_url', 'name', 'platform', 'model', 'task']);
const NAVIGATE_DEEP_LINK_PARAMS = new Set(['route']);

const allowedDeepLinkParams = (action: string): ReadonlySet<string> => {
  if (action === 'add-provider' || action === 'provider/add') return ADD_PROVIDER_DEEP_LINK_PARAMS;
  if (action === 'navigate') return NAVIGATE_DEEP_LINK_PARAMS;
  return new Set();
};

const isSafeDeepLinkBaseUrl = (value: string): boolean => {
  try {
    const parsed = new URL(value);
    return (
      (parsed.protocol === 'https:' || parsed.protocol === 'http:') &&
      parsed.hostname.length > 0 &&
      parsed.username.length === 0 &&
      parsed.password.length === 0 &&
      parsed.search.length === 0 &&
      parsed.hash.length === 0
    );
  } catch {
    return false;
  }
};

/**
 * Parse only non-sensitive deep-link suggestions. Provider credentials must
 * never travel in a URL; users enter them directly in the provider form.
 */
export function parseDeepLink(url: string): { action: string; params: Record<string, string> } {
  try {
    const u = new URL(url);
    const action = u.hostname || u.pathname.replace(/^\/+/, '');
    const allowed = allowedDeepLinkParams(action);
    const params: Record<string, string> = {};
    u.searchParams.forEach((value, key) => {
      if (!allowed.has(key)) return;
      const normalized = value.trim();
      if (!normalized) return;
      if (key === 'base_url' && !isSafeDeepLinkBaseUrl(normalized)) return;
      if (key === 'route' && /[?#]/u.test(normalized)) return;
      params[key] = normalized;
    });
    return { action, params };
  } catch {
    return { action: '', params: {} };
  }
}

/**
 * Subscribe to `nomifun://` deep links. The Rust shell (apps/desktop/src/main.rs)
 * forwards opened URLs on the Tauri event `deep-link://received` as a string[].
 */
export async function subscribeDeepLink(
  callback: (payload: { action: string; params: Record<string, string> }) => void
): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  return listen<string[]>('deep-link://received', (event) => {
    for (const url of event.payload ?? []) callback(parseDeepLink(url));
  });
}

// ---- window controls (@tauri-apps/api/window) ----

export async function tauriWindowMinimize(): Promise<void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().minimize();
}
export async function tauriWindowMaximize(): Promise<void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().maximize();
}
export async function tauriWindowUnmaximize(): Promise<void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().unmaximize();
}
export async function tauriWindowToggleMaximize(): Promise<void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().toggleMaximize();
}
export async function tauriWindowClose(): Promise<void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().close();
}
export async function tauriWindowIsMaximized(): Promise<boolean> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  return getCurrentWindow().isMaximized();
}
export async function subscribeWindowMaximized(
  callback: (payload: { is_maximized: boolean }) => void
): Promise<() => void> {
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const win = getCurrentWindow();
  return win.onResized(() => {
    void win.isMaximized().then((is_maximized) => callback({ is_maximized }));
  });
}

// ---- WebUI / LAN remote-access lifecycle (Tauri commands + status event) ----

/** Invoke a Tauri command via `@tauri-apps/api/core`. */
async function invokeCommand<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

/** Current WebUI/LAN serving status (backend `webui_get_status`). */
export function tauriWebuiGetStatus<T>(): Promise<T> {
  return invokeCommand<T>('webui_get_status');
}

/** Start LAN serving (backend `webui_start`). Returns the resulting status. */
export function tauriWebuiStart<T>(): Promise<T> {
  return invokeCommand<T>('webui_start');
}

/** Stop LAN serving (backend `webui_stop`). Returns the resulting status. */
export function tauriWebuiStop<T>(): Promise<T> {
  return invokeCommand<T>('webui_stop');
}

export interface TauriRelayPairingBootstrapRequest {
  pairingEnvelope: string;
}

export type TauriRelayPairingState = 'disconnected' | 'connecting' | 'connected' | 'error';

export interface TauriRelayPairingStatus {
  state: TauriRelayPairingState;
  pairUrl?: string;
  relay?: string;
  businessUrl?: string;
  inviteId?: string;
  tunnelId?: string;
  tunnelSlug?: string;
  agentStateDir?: string;
  agentPid?: number;
  qrExpiresAtMs?: number;
  webuiPort?: number;
  error?: string;
}

/** Bootstrap a desktop Relay agent from a one-time pairing envelope. */
export function tauriRelayPairingBootstrap(
  request: TauriRelayPairingBootstrapRequest
): Promise<TauriRelayPairingStatus> {
  return invokeCommand<TauriRelayPairingStatus>('relay_pairing_bootstrap', {
    pairingEnvelope: request.pairingEnvelope,
  });
}

/** Read the local Relay agent pairing state without returning credentials. */
export function tauriRelayPairingGetStatus(): Promise<TauriRelayPairingStatus> {
  return invokeCommand<TauriRelayPairingStatus>('relay_pairing_get_status');
}

/** Stop the local Relay agent, if one is running. */
export function tauriRelayPairingStop(): Promise<TauriRelayPairingStatus> {
  return invokeCommand<TauriRelayPairingStatus>('relay_pairing_stop');
}

/** Restart the local Relay agent using its persisted long-lived credential. */
export function tauriRelayPairingRestart(): Promise<TauriRelayPairingStatus> {
  return invokeCommand<TauriRelayPairingStatus>('relay_pairing_restart');
}

/** Stop the agent and remove all local Relay pairing state. */
export function tauriRelayPairingDisconnect(): Promise<TauriRelayPairingStatus> {
  return invokeCommand<TauriRelayPairingStatus>('relay_pairing_disconnect');
}

/**
 * Subscribe to backend-emitted WebUI/LAN status changes
 * (`apps/desktop/src/main.rs` forwards them on `webui://status-changed`).
 */
export async function subscribeWebuiStatus<T>(callback: (status: T) => void): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  return listen<T>('webui://status-changed', (event) => callback(event.payload));
}
