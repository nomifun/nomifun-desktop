/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import type { AgentPresetId } from '@/common/types/ids';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { isExecutableAgentPreset } from './agentSelectionUtils';
import type { ExecutableAgentPreset } from '../types';

export type GuidAgentSelectionResult = {
  selectedPresetId: string;
  selectedPreset: ExecutableAgentPreset | undefined;
  presets: ExecutableAgentPreset[];
  isLoading: boolean;
  setSelectedPresetId: (presetId: string) => void;
  refreshPresets: () => Promise<void>;
};

type UseGuidAgentSelectionOptions = {
  resetAgentSelection?: boolean;
  selectedAgentPresetId?: string;
  locationKey?: string;
};

const readSavedPresetId = (): string =>
  configService.get('guid.lastSelectedAgentPreset') || '';

const saveSelectedPresetId = (presetId: string): void => {
  void configService
    .set('guid.lastSelectedAgentPreset', presetId as AgentPresetId)
    .catch((error) => {
      console.error('Failed to save selected AgentPreset:', error);
    });
};

/** Selects executable user AgentPresets saved by the Agent Workbench. */
export const useGuidAgentSelection = ({
  resetAgentSelection = false,
  selectedAgentPresetId,
  locationKey,
}: UseGuidAgentSelectionOptions): GuidAgentSelectionResult => {
  const [selectedPresetId, setSelectedPresetIdState] = useState<string>(() => {
    try {
      return readSavedPresetId();
    } catch {
      return '';
    }
  });

  const {
    presets: savedPresets,
    isLoading,
    refresh: refreshPresets,
  } = useAgentPresets();
  const presets = useMemo(
    () => savedPresets.filter(isExecutableAgentPreset),
    [savedPresets]
  );

  const setSelectedPresetId = useCallback((presetId: string) => {
    setSelectedPresetIdState(presetId);
    saveSelectedPresetId(presetId);
  }, []);

  const selectedPreset = useMemo(
    () => presets.find((preset) => preset.preset_id === selectedPresetId),
    [presets, selectedPresetId]
  );

  const resetHandledRef = useRef(false);
  const previousLocationKeyRef = useRef(locationKey);
  if (locationKey !== previousLocationKeyRef.current) {
    previousLocationKeyRef.current = locationKey;
    resetHandledRef.current = false;
  }

  useLayoutEffect(() => {
    if (isLoading || presets.length === 0 || resetHandledRef.current) return;

    if (
      selectedAgentPresetId &&
      presets.some((preset) => preset.preset_id === selectedAgentPresetId)
    ) {
      resetHandledRef.current = true;
      setSelectedPresetIdState(selectedAgentPresetId);
      saveSelectedPresetId(selectedAgentPresetId);
      return;
    }

    if (resetAgentSelection) {
      resetHandledRef.current = true;
      const firstPresetId = presets[0].preset_id;
      setSelectedPresetIdState(firstPresetId);
      saveSelectedPresetId(firstPresetId);
    }
  }, [isLoading, presets, resetAgentSelection, selectedAgentPresetId]);

  useEffect(() => {
    if (isLoading) return;
    if (presets.length === 0) {
      setSelectedPresetIdState('');
      return;
    }
    if (resetAgentSelection) return;
    if (
      selectedAgentPresetId &&
      presets.some((preset) => preset.preset_id === selectedAgentPresetId)
    ) {
      return;
    }

    const savedPresetId = readSavedPresetId();
    if (
      savedPresetId &&
      presets.some((preset) => preset.preset_id === savedPresetId)
    ) {
      setSelectedPresetIdState(savedPresetId);
      return;
    }

    const firstPresetId = presets[0].preset_id;
    setSelectedPresetIdState(firstPresetId);
    if (savedPresetId !== firstPresetId) {
      saveSelectedPresetId(firstPresetId);
    }
  }, [isLoading, presets, resetAgentSelection, selectedAgentPresetId]);

  return {
    selectedPresetId,
    selectedPreset,
    presets,
    isLoading,
    setSelectedPresetId,
    refreshPresets,
  };
};
