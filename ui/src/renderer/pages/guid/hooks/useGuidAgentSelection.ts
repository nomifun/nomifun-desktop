/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import type { AgentPresetSummary, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { isExecutableAgentPreset } from './agentSelectionUtils';
import type {
  ExecutableAgentPreset,
  GuidAgentSelection,
} from '../types';

export type GuidAgentSelectionResult = {
  selection: GuidAgentSelection;
  selectedPreset: ExecutableAgentPreset | undefined;
  presets: ExecutableAgentPreset[];
  draftPresets: AgentPresetSummary[];
  officialTemplates: OfficialPresetTemplate[];
  isLoading: boolean;
  isLoaded: boolean;
  loadError: Error | undefined;
  setSelection: (selection: GuidAgentSelection) => void;
  selectDefaultAgent: () => void;
  refreshPresets: () => Promise<void>;
};

type UseGuidAgentSelectionOptions = {
  resetAgentSelection?: boolean;
  selectedAgentPresetId?: string;
  locationKey?: string;
};

const DEFAULT_AGENT_SELECTION: GuidAgentSelection = { kind: 'default' };

const isGuidAgentSelection = (
  value: unknown
): value is GuidAgentSelection => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  return (
    candidate.kind === 'default' ||
    (candidate.kind === 'preset' && typeof candidate.presetId === 'string')
  );
};

const readSavedSelection = (): GuidAgentSelection => {
  const saved: unknown = configService.get('guid.agentSelection');
  return isGuidAgentSelection(saved) ? saved : DEFAULT_AGENT_SELECTION;
};

const saveSelection = (selection: GuidAgentSelection): void => {
  void configService
    .set('guid.agentSelection', selection)
    .catch((error) => {
      console.error('Failed to save Guid Agent selection:', error);
    });
};

/** Selects plain Nomi or an executable user AgentPreset from the Workbench. */
export const useGuidAgentSelection = ({
  resetAgentSelection = false,
  selectedAgentPresetId,
  locationKey,
}: UseGuidAgentSelectionOptions): GuidAgentSelectionResult => {
  const [selection, setSelectionState] = useState<GuidAgentSelection>(() => {
    try {
      return readSavedSelection();
    } catch {
      return DEFAULT_AGENT_SELECTION;
    }
  });

  const {
    library,
    presets: savedPresets,
    isLoading,
    error: loadError,
    refresh: refreshPresets,
  } = useAgentPresets();
  const isLoaded = library !== undefined;
  const presets = useMemo(
    () => savedPresets.filter(isExecutableAgentPreset),
    [savedPresets]
  );
  const draftPresets = useMemo(
    () => savedPresets.filter((preset) => !isExecutableAgentPreset(preset)),
    [savedPresets]
  );

  const setSelection = useCallback((nextSelection: GuidAgentSelection) => {
    setSelectionState(nextSelection);
    saveSelection(nextSelection);
  }, []);

  const selectDefaultAgent = useCallback(() => {
    setSelection(DEFAULT_AGENT_SELECTION);
  }, [setSelection]);

  const selectedPresetId =
    selection.kind === 'preset' ? selection.presetId : undefined;
  const selectedPreset = useMemo(
    () =>
      selectedPresetId
        ? presets.find((preset) => preset.preset_id === selectedPresetId)
        : undefined,
    [presets, selectedPresetId]
  );
  const effectiveSelection =
    selection.kind === 'preset' && !selectedPreset && loadError
      ? DEFAULT_AGENT_SELECTION
      : selection;

  const navigationRequestHandledRef = useRef(false);
  const previousLocationKeyRef = useRef(locationKey);
  if (locationKey !== previousLocationKeyRef.current) {
    previousLocationKeyRef.current = locationKey;
    navigationRequestHandledRef.current = false;
  }

  useLayoutEffect(() => {
    if (navigationRequestHandledRef.current) return;

    if (resetAgentSelection) {
      navigationRequestHandledRef.current = true;
      selectDefaultAgent();
      return;
    }

    if (!selectedAgentPresetId || isLoading || !isLoaded) return;

    const preset = presets.find(
      (candidate) => candidate.preset_id === selectedAgentPresetId
    );
    if (!preset && loadError) return;

    navigationRequestHandledRef.current = true;
    if (preset) {
      setSelection({ kind: 'preset', presetId: preset.preset_id });
      return;
    }

    selectDefaultAgent();
  }, [
    isLoading,
    isLoaded,
    loadError,
    presets,
    resetAgentSelection,
    selectDefaultAgent,
    selectedAgentPresetId,
    setSelection,
  ]);

  useEffect(() => {
    if (
      isLoading ||
      !isLoaded ||
      loadError ||
      resetAgentSelection ||
      selectedAgentPresetId ||
      selection.kind === 'default' ||
      selectedPreset
    ) {
      return;
    }
    selectDefaultAgent();
  }, [
    isLoading,
    isLoaded,
    loadError,
    resetAgentSelection,
    selectedAgentPresetId,
    selectedPreset,
    selectDefaultAgent,
    selection.kind,
  ]);

  return {
    selection: effectiveSelection,
    selectedPreset,
    presets,
    draftPresets,
    officialTemplates: library?.official_templates ?? [],
    isLoading,
    isLoaded,
    loadError,
    setSelection,
    selectDefaultAgent,
    refreshPresets,
  };
};
