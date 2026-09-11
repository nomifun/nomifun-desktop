/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import type { AgentPresetSummary, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import {
  DEFAULT_GUID_AGENT_SELECTION,
  isExecutableAgentPreset,
  normalizeGuidAgentSelection,
} from './agentSelectionUtils';
import type {
  ExecutableAgentPreset,
  GuidAgentSelection,
} from '../types';

export type GuidAgentSelectionResult = {
  selection: GuidAgentSelection;
  selectedPreset: ExecutableAgentPreset | undefined;
  selectedTemplate: OfficialPresetTemplate | undefined;
  presets: ExecutableAgentPreset[];
  draftPresets: AgentPresetSummary[];
  officialTemplates: OfficialPresetTemplate[];
  isLoading: boolean;
  isLoaded: boolean;
  loadError: Error | undefined;
  setSelection: (selection: GuidAgentSelection) => void;
  refreshPresets: () => Promise<void>;
};

type UseGuidAgentSelectionOptions = {
  resetAgentSelection?: boolean;
  selectedAgentPresetId?: string;
  locationKey?: string;
};

const readSavedSelection = (): GuidAgentSelection => {
  const saved: unknown = configService.get('guid.agentSelection');
  return normalizeGuidAgentSelection(saved);
};

const saveSelection = (selection: GuidAgentSelection): void => {
  void configService
    .set('guid.agentSelection', selection)
    .catch((error) => {
      console.error('Failed to save Guid Agent selection:', error);
    });
};

/** Selects an official Agent or a saved personal Agent from the workbench catalog. */
export const useGuidAgentSelection = ({
  resetAgentSelection = false,
  selectedAgentPresetId,
  locationKey,
}: UseGuidAgentSelectionOptions): GuidAgentSelectionResult => {
  const [selection, setSelectionState] = useState<GuidAgentSelection>(() => {
    try {
      return readSavedSelection();
    } catch {
      return DEFAULT_GUID_AGENT_SELECTION;
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

  const selectDefaultTemplate = useCallback(() => {
    setSelection(DEFAULT_GUID_AGENT_SELECTION);
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
      ? DEFAULT_GUID_AGENT_SELECTION
      : selection;
  const selectedTemplate = effectiveSelection.kind === 'template'
    ? library?.official_templates.find(
        (template) => template.template_key === effectiveSelection.templateKey
      )
    : undefined;

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
      selectDefaultTemplate();
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

    selectDefaultTemplate();
  }, [
    isLoading,
    isLoaded,
    loadError,
    presets,
    resetAgentSelection,
    selectDefaultTemplate,
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
      selectedPreset ||
      selectedTemplate
    ) {
      return;
    }
    selectDefaultTemplate();
  }, [
    isLoading,
    isLoaded,
    loadError,
    resetAgentSelection,
    selectedAgentPresetId,
    selectedPreset,
    selectedTemplate,
    selectDefaultTemplate,
  ]);

  return {
    selection: effectiveSelection,
    selectedPreset,
    selectedTemplate,
    presets,
    draftPresets,
    officialTemplates: library?.official_templates ?? [],
    isLoading,
    isLoaded,
    loadError,
    setSelection,
    refreshPresets,
  };
};
