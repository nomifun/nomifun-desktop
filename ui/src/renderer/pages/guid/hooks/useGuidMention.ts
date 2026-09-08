/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  ExecutableAgentPreset,
  GuidAgentSelection,
  MentionOption,
} from '../types';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { OfficialPresetTemplate } from '@/common/types/agentPlatform';
import { TEMPLATE_I18N_PATH } from '../../agentSettings/model';

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
  officialTemplates?: OfficialPresetTemplate[];
  selection: GuidAgentSelection;
  setSelection: (selection: GuidAgentSelection) => void;
  selectedPreset: ExecutableAgentPreset | undefined;
  defaultAgentLabel: string;
  setInput: React.Dispatch<React.SetStateAction<string>>;
};

const selectionKey = (selection: GuidAgentSelection): string =>
  selection.kind === 'default'
    ? 'guid-agent-default'
    : selection.kind === 'template'
      ? `guid-agent-template:${selection.templateKey}`
      : `guid-agent-preset:${selection.presetId}`;

/** Manages plain Nomi and AgentPreset selection through the Guid @ mention UI. */
export const useGuidMention = ({
  presets,
  officialTemplates = [],
  selection,
  setSelection,
  selectedPreset,
  defaultAgentLabel,
  setInput,
}: UseGuidMentionOptions): GuidMentionResult => {
  const { t } = useTranslation();
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [mentionOpen, setMentionOpen] = useState(false);
  const [mentionSelectorVisible, setMentionSelectorVisible] = useState(false);
  const [mentionSelectorOpen, setMentionSelectorOpen] = useState(false);
  const [mentionActiveIndex, setMentionActiveIndex] = useState(0);
  const mentionMenuRef = useRef<HTMLDivElement>(null);
  const mentionMatchRegex = useMemo(() => /(?:^|\s)@([^\s@]*)$/, []);

  const mentionOptions = useMemo(
    () => [
      {
        key: selectionKey({ kind: 'default' }),
        label: defaultAgentLabel,
        tokens: new Set([
          defaultAgentLabel.toLowerCase(),
          'nomi',
          'default',
        ]),
        selection: { kind: 'default' } as const,
        avatarEmoji: undefined,
        avatarImage: undefined,
        logo: undefined,
      },
      ...officialTemplates.map((template) => {
        const label = t(`agentSettings.template.${TEMPLATE_I18N_PATH[template.template_key]}.name`);
        const templateSelection: GuidAgentSelection = { kind: 'template', templateKey: template.template_key };
        return {
          key: selectionKey(templateSelection), label,
          tokens: new Set([label.toLowerCase(), template.template_key]),
          selection: templateSelection,
          avatarEmoji: undefined, avatarImage: undefined, logo: undefined,
        };
      }),
      ...presets.map((preset) => {
        const label = preset.display_name;
        const normalizedLabel = label.toLowerCase();
        const presetSelection: GuidAgentSelection = {
          kind: 'preset',
          presetId: preset.preset_id,
        };
        return {
          key: selectionKey(presetSelection),
          label,
          tokens: new Set([
            normalizedLabel,
            normalizedLabel.replace(/\s+/g, '-'),
            normalizedLabel.replace(/\s+/g, ''),
            preset.preset_id.toLowerCase(),
          ]),
          selection: presetSelection,
          avatarEmoji: undefined,
          avatarImage: undefined,
          logo: undefined,
        };
      }),
    ],
    [defaultAgentLabel, presets, officialTemplates, t]
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
      const option = mentionOptions.find((candidate) => candidate.key === key);
      if (!option) return;
      setSelection(option.selection);
      setInput((previous) => stripMentionToken(previous));
      setMentionOpen(false);
      setMentionSelectorOpen(false);
      setMentionSelectorVisible(true);
      setMentionQuery(null);
      setMentionActiveIndex(0);
    },
    [mentionOptions, setInput, setSelection, stripMentionToken]
  );

  const selectedAgentLabel =
    selection.kind === 'default'
      ? defaultAgentLabel
      : selection.kind === 'template'
        ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[selection.templateKey]}.name`)
        : selectedPreset?.display_name ?? defaultAgentLabel;
  const selectedKey = selectionKey(selection);
  const mentionMenuActiveOption =
    filteredMentionOptions[mentionActiveIndex] || filteredMentionOptions[0];
  const mentionMenuSelectedKey =
    mentionOpen || mentionSelectorOpen
      ? mentionMenuActiveOption?.key || selectedKey
      : selectedKey;

  useEffect(() => {
    if (mentionOpen) {
      setMentionActiveIndex(0);
      return;
    }
    if (mentionSelectorOpen) {
      const selectedIndex = filteredMentionOptions.findIndex(
        (option) => option.key === selectedKey
      );
      setMentionActiveIndex(selectedIndex >= 0 ? selectedIndex : 0);
    }
  }, [
    filteredMentionOptions,
    mentionOpen,
    mentionQuery,
    mentionSelectorOpen,
    selectedKey,
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
