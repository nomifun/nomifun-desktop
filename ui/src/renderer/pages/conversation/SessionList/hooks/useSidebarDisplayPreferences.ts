/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useState } from 'react';

import {
  getPresetSidebarDisplayPreferences,
  normalizeSidebarDisplayPreferences,
  withCustomSidebarDisplayPreference,
  type SidebarDisplayPreferences,
  type SidebarDisplayPreset,
} from '../utils/sidebarDisplayPreferences';

const STORAGE_KEY = 'nomifun:session-sidebar-display-preferences';
const CHANGE_EVENT = 'nomifun:session-sidebar-display-preferences-change';

type PresetOption = Exclude<SidebarDisplayPreset, 'custom'>;
type PreferencePatch = Partial<Omit<SidebarDisplayPreferences, 'preset'>>;

function readPreferences(): SidebarDisplayPreferences {
  if (typeof localStorage === 'undefined') return normalizeSidebarDisplayPreferences(undefined);
  try {
    return normalizeSidebarDisplayPreferences(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? 'null'));
  } catch {
    return normalizeSidebarDisplayPreferences(undefined);
  }
}

function persistPreferences(preferences: SidebarDisplayPreferences) {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(preferences));
}

function dispatchPreferencesChange(preferences: SidebarDisplayPreferences) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent<SidebarDisplayPreferences>(CHANGE_EVENT, { detail: preferences }));
}

export function useSidebarDisplayPreferences() {
  const [preferences, setPreferencesState] = useState<SidebarDisplayPreferences>(readPreferences);

  const commitPreferences = useCallback((next: SidebarDisplayPreferences) => {
    const normalized = normalizeSidebarDisplayPreferences(next);
    persistPreferences(normalized);
    setPreferencesState(normalized);
    dispatchPreferencesChange(normalized);
  }, []);

  const applyPreset = useCallback(
    (preset: PresetOption) => {
      commitPreferences(getPresetSidebarDisplayPreferences(preset));
    },
    [commitPreferences]
  );

  const updatePreference = useCallback(
    (patch: PreferencePatch) => {
      setPreferencesState((current) => {
        const next = withCustomSidebarDisplayPreference(current, patch);
        persistPreferences(next);
        dispatchPreferencesChange(next);
        return next;
      });
    },
    []
  );

  useEffect(() => {
    if (typeof window === 'undefined') return undefined;

    const handleStorage = (event: StorageEvent) => {
      if (event.key === STORAGE_KEY) setPreferencesState(readPreferences());
    };
    const handleCustom = (event: Event) => {
      setPreferencesState(normalizeSidebarDisplayPreferences((event as CustomEvent).detail));
    };

    window.addEventListener('storage', handleStorage);
    window.addEventListener(CHANGE_EVENT, handleCustom);
    return () => {
      window.removeEventListener('storage', handleStorage);
      window.removeEventListener(CHANGE_EVENT, handleCustom);
    };
  }, []);

  return {
    preferences,
    applyPreset,
    updatePreference,
  };
}
