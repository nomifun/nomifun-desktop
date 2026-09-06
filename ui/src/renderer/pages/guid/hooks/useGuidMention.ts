/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentPresetSummary } from '@/common/types/agentPlatform';
import type { ExecutableAgentPreset, MentionOption } from '../types';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

export type GuidMentionResult = {
  mentionQuery: string | null;
  setMentionQuery: React.Dispatch<React.SetStateAction<string | null>>;
  mentionOpen: boolean;
  setMentionOpen: React.Dispatch<React.SetStateAction<boolean>>;
  mentionSelectorVisible: boolean;
  setMentionSelectorVisible: React.Dispatch<React.SetStateAction<boolean>>;
  mentionSelectorOpen: boolean;
  setMentionSelectorOpen: React.Dispatch<React.SetStateAction<boolean>>;
  mentionActiveIndex: number;
  setMentionActiveIndex: React.Dispatch<React.SetStateAction<number>>;
  mentionOptions: MentionOption[];
  filteredMentionOptions: MentionOption[];
  selectMentionAgent: (key: string) => void;
  mentionMenuRef: React.RefObject<HTMLDivElement | null>;
  mentionMatchRegex: RegExp;
  selectedAgentLabel: string;
  mentionMenuSelectedKey: string;
};

type UseGuidMentionOptions = {
  presets: ExecutableAgentPreset[];
  selectedPresetId: string;
  setSelectedPresetId: (presetId: string) => void;
  selectedPreset: AgentPresetSummary | undefined;
  setInput: React.Dispatch<React.SetStateAction<string>>;
};

/** Manages AgentPreset selection through the Guid @ mention UI. */
export const useGuidMention = ({
  presets,
  selectedPresetId,
  setSelectedPresetId,
  selectedPreset,
  setInput,
}: UseGuidMentionOptions): GuidMentionResult => {
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [mentionOpen, setMentionOpen] = useState(false);
  const [mentionSelectorVisible, setMentionSelectorVisible] = useState(false);
  const [mentionSelectorOpen, setMentionSelectorOpen] = useState(false);
  const [mentionActiveIndex, setMentionActiveIndex] = useState(0);
  const mentionMenuRef = useRef<HTMLDivElement>(null);
  const mentionMatchRegex = useMemo(() => /(?:^|\s)@([^\s@]*)$/, []);

  const mentionOptions = useMemo(
    () =>
      presets.map((preset) => {
        const label = preset.display_name;
        const normalizedLabel = label.toLowerCase();
        return {
          key: preset.preset_id,
          label,
          tokens: new Set([
            normalizedLabel,
            normalizedLabel.replace(/\s+/g, '-'),
            normalizedLabel.replace(/\s+/g, ''),
            preset.preset_id.toLowerCase(),
          ]),
          avatarEmoji: undefined,
          avatarImage: undefined,
          logo: undefined,
        };
      }),
    [presets]
  );

  const filteredMentionOptions = useMemo(() => {
    if (!mentionQuery) return mentionOptions;
    const query = mentionQuery.toLowerCase();
    return mentionOptions.filter((option) =>
      Array.from(option.tokens).some((token) => token.startsWith(query))
    );
  }, [mentionOptions, mentionQuery]);

  const stripMentionToken = useCallback(
    (value: string) => {
      if (!mentionMatchRegex.test(value)) return value;
      return value.replace(mentionMatchRegex, '').trimEnd();
    },
    [mentionMatchRegex]
  );

  const selectMentionAgent = useCallback(
    (key: string) => {
      setSelectedPresetId(key);
      setInput((previous) => stripMentionToken(previous));
      setMentionOpen(false);
      setMentionSelectorOpen(false);
      setMentionSelectorVisible(true);
      setMentionQuery(null);
      setMentionActiveIndex(0);
    },
    [setInput, setSelectedPresetId, stripMentionToken]
  );

  const selectedAgentLabel = selectedPreset?.display_name || selectedPresetId;
  const mentionMenuActiveOption =
    filteredMentionOptions[mentionActiveIndex] || filteredMentionOptions[0];
  const mentionMenuSelectedKey =
    mentionOpen || mentionSelectorOpen
      ? mentionMenuActiveOption?.key || selectedPresetId
      : selectedPresetId;

  useEffect(() => {
    if (mentionOpen) {
      setMentionActiveIndex(0);
      return;
    }
    if (mentionSelectorOpen) {
      const selectedIndex = filteredMentionOptions.findIndex(
        (option) => option.key === selectedPresetId
      );
      setMentionActiveIndex(selectedIndex >= 0 ? selectedIndex : 0);
    }
  }, [
    filteredMentionOptions,
    mentionOpen,
    mentionQuery,
    mentionSelectorOpen,
    selectedPresetId,
  ]);

  useEffect(() => {
    if (!mentionOpen && !mentionSelectorOpen) return;
    const container = mentionMenuRef.current;
    if (!container) return;
    const target = container.querySelector<HTMLElement>(
      `[data-mention-index="${mentionActiveIndex}"]`
    );
    if (!target) return;
    target.scrollIntoView({ block: 'nearest' });
  }, [mentionActiveIndex, mentionOpen, mentionSelectorOpen]);

  return {
    mentionQuery,
    setMentionQuery,
    mentionOpen,
    setMentionOpen,
    mentionSelectorVisible,
    setMentionSelectorVisible,
    mentionSelectorOpen,
    setMentionSelectorOpen,
    mentionActiveIndex,
    setMentionActiveIndex,
    mentionOptions,
    filteredMentionOptions,
    selectMentionAgent,
    mentionMenuRef,
    mentionMatchRegex,
    selectedAgentLabel,
    mentionMenuSelectedKey,
  };
};
