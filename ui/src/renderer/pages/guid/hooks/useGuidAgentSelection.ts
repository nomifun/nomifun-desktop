/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useAgentPresets } from '@/renderer/hooks/agent/useAgentPresets';
import type { AgentPresetSummary, OfficialPresetKey, OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import { useGuidDraftState } from './useGuidDraftState';
import {
  DEFAULT_GUID_AGENT_SELECTION,
  isExecutableAgentPreset,
  normalizeGuidAgentSelection,
  readGuidDefaultAgentSelection,
} from './agentSelectionUtils';
import type {
  ExecutableAgentPreset,
  GuidAgentSelection,
} from '../types';
import {
  filterConversationAgentPresets,
  isConversationAgentTemplate,
} from '@/renderer/components/agent/conversationAgentCatalog';

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
  selectedAgentTemplateKey?: OfficialPresetKey;
  locationKey?: string;
};

/** Selects an official Agent or a saved personal Agent from the workbench catalog. */
export const useGuidAgentSelection = ({
  resetAgentSelection = false,
  selectedAgentPresetId,
  selectedAgentTemplateKey,
  locationKey,
}: UseGuidAgentSelectionOptions): GuidAgentSelectionResult => {
  const [selection, setSelectionState] = useGuidDraftState<GuidAgentSelection>('agent', () => {
    try {
      return readGuidDefaultAgentSelection();
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
  const conversationPresets = useMemo(
    () => filterConversationAgentPresets(savedPresets, library?.active_bindings ?? []),
    [library?.active_bindings, savedPresets],
  );
  const presets = useMemo(
    () => conversationPresets.filter(isExecutableAgentPreset),
    [conversationPresets]
  );
  const draftPresets = useMemo(
    () => conversationPresets.filter((preset) => !isExecutableAgentPreset(preset)),
    [conversationPresets]
  );
  const officialTemplates = useMemo(
    () => (library?.official_templates ?? []).filter(isConversationAgentTemplate),
    [library?.official_templates],
  );

  const setSelection = useCallback((nextSelection: GuidAgentSelection) => {
    const normalized = normalizeGuidAgentSelection(nextSelection);
    setSelectionState(normalized);
  }, [setSelectionState]);

  const selectDefaultAgent = useCallback(() => {
    const configured = readGuidDefaultAgentSelection();
    const available = configured.kind === 'preset'
      ? presets.some((preset) => preset.preset_id === configured.presetId)
      : officialTemplates.some(
          (template) => template.template_key === configured.templateKey
        );
    setSelectionState(available ? configured : DEFAULT_GUID_AGENT_SELECTION);
  }, [officialTemplates, presets, setSelectionState]);

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
    ? officialTemplates.find(
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
      if (!isLoaded) {
        setSelectionState(readGuidDefaultAgentSelection());
        return;
      }
      navigationRequestHandledRef.current = true;
      selectDefaultAgent();
      return;
    }

    if (isLoading || !isLoaded) return;

    if (selectedAgentTemplateKey) {
      const template = officialTemplates.find(
        (candidate) => candidate.template_key === selectedAgentTemplateKey
      );
      if (!template && loadError) return;

      navigationRequestHandledRef.current = true;
      if (template) {
        setSelection({ kind: 'template', templateKey: template.template_key });
        return;
      }

      selectDefaultAgent();
      return;
    }

    if (!selectedAgentPresetId) return;

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
    officialTemplates,
    resetAgentSelection,
    selectDefaultAgent,
    selectedAgentPresetId,
    selectedAgentTemplateKey,
    setSelection,
    setSelectionState,
  ]);

  useEffect(() => {
    if (
      isLoading ||
      !isLoaded ||
      loadError ||
      resetAgentSelection ||
      selectedAgentPresetId ||
      selectedAgentTemplateKey ||
      selectedPreset ||
      selectedTemplate
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
    selectedAgentTemplateKey,
    selectedPreset,
    selectedTemplate,
    selectDefaultAgent,
  ]);

  return {
    selection: effectiveSelection,
    selectedPreset,
    selectedTemplate,
    presets,
    draftPresets,
    officialTemplates,
    isLoading,
    isLoaded,
    loadError,
    setSelection,
    refreshPresets,
  };
};
