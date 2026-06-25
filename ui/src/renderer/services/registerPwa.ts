/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isDesktopShell } from '@renderer/utils/platform';

const SERVICE_WORKER_URL = './sw.js';
const LOCALHOST_HOSTS = new Set(['localhost', '127.0.0.1', '::1']);

function serviceWorkerAvailable(): boolean {
  return typeof window !== 'undefined' && typeof navigator !== 'undefined' && 'serviceWorker' in navigator;
}

function isPwaRegistrationSupported(): boolean {
  if (!serviceWorkerAvailable()) {
    return false;
  }

  // Never run a service worker inside the bundled desktop shell — Electron OR
  // Tauri. The desktop serves its frontend from local embedded assets, so a SW
  // adds no offline value; worse, its cache-first shell pins a stale index.html
  // and asset hashes across rebuilds, which surfaces as a white screen after the
  // embedded frontend changes. Keyed on isDesktopShell() (the __backendPort
  // signal common to both shells), NOT isElectronDesktop() — under Tauri the
  // latter is false, which is exactly how the SW slipped into the Tauri build.
  if (isDesktopShell()) {
    return false;
  }

  const { protocol, hostname } = window.location;
  const isHttpOrigin = protocol === 'http:' || protocol === 'https:';
  if (!isHttpOrigin) {
    return false;
  }

  return window.isSecureContext || LOCALHOST_HOSTS.has(hostname);
}

/**
 * Tear down any previously-registered service worker and its caches. Runs in the
 * desktop shell so installs that registered a SW before the desktop guard
 * existed self-heal: a poisoned shell cache otherwise shows as a permanent white
 * screen after the embedded frontend is rebuilt (the cached index.html points at
 * asset hashes the new binary no longer contains).
 */
async function purgeServiceWorkers(): Promise<void> {
  if (!serviceWorkerAvailable()) {
    return;
  }
  try {
    const registrations = await navigator.serviceWorker.getRegistrations();
    await Promise.all(registrations.map((registration) => registration.unregister().catch((): false => false)));
  } catch {
    // best-effort cleanup
  }
  try {
    if (typeof caches !== 'undefined') {
      const keys = await caches.keys();
      await Promise.all(keys.map((key) => caches.delete(key).catch((): false => false)));
    }
  } catch {
    // best-effort cleanup
  }
}

export async function registerPwa(): Promise<ServiceWorkerRegistration | undefined> {
  // Desktop shell (Electron or Tauri): actively remove any pre-existing SW +
  // caches and never register one. See purgeServiceWorkers / the guard above.
  if (typeof window !== 'undefined' && isDesktopShell()) {
    await purgeServiceWorkers();
    return undefined;
  }

  if (!isPwaRegistrationSupported()) {
    return undefined;
  }

  try {
    const registration = await navigator.serviceWorker.register(SERVICE_WORKER_URL, { scope: './' });
    // Poll for updates on every page load so a fixed SW (e.g. v2 replacing
    // a poisoned v1 cache) reaches users without waiting for the browser's
    // own 24h update heuristic.
    registration.update().catch((): undefined => undefined);
    return registration;
  } catch (error) {
    console.warn('[PWA] Failed to register service worker:', error);
    return undefined;
  }
}
