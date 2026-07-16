/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useSyncExternalStore } from 'react';

export interface UpdateAvailabilitySnapshot {
  available: boolean;
  version?: string;
}

const NO_UPDATE: UpdateAvailabilitySnapshot = { available: false };

let snapshot: UpdateAvailabilitySnapshot = NO_UPDATE;
const listeners = new Set<() => void>();

const emit = () => listeners.forEach((listener) => listener());

const setSnapshot = (next: UpdateAvailabilitySnapshot) => {
  if (snapshot.available === next.available && snapshot.version === next.version) return;
  snapshot = next;
  emit();
};

/** Publish a successful update check so every app-level entry point stays in sync. */
export const reportUpdateAvailable = (version?: string) => {
  setSnapshot({ available: true, ...(version ? { version } : {}) });
};

/** Hide the app-level update entry after an authoritative check finds no update. */
export const reportNoUpdateAvailable = () => {
  setSnapshot(NO_UPDATE);
};

const subscribe = (listener: () => void): (() => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

/** Non-React snapshot getter, also useful for focused store tests. */
export const getUpdateAvailabilitySnapshot = (): UpdateAvailabilitySnapshot => snapshot;

/** Shared, renderer-local update availability for persistent global UI. */
export const useUpdateAvailability = (): UpdateAvailabilitySnapshot =>
  useSyncExternalStore(subscribe, getUpdateAvailabilitySnapshot, getUpdateAvailabilitySnapshot);
