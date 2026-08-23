/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ModelTask, ModelTrait } from '@/common/config/storage';
import type { ProviderId } from '@/common/types/ids';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { capabilityOf } from '@/common/utils/providerModels';
import { modelDisplayLabel } from '@/common/utils/modelPresentation';
import {
  taskModelSelectState,
  type TaskModelProviderScope,
  type TaskModelSelection,
} from './taskModelSelectState';
import { ttsUsesModelIdAsVoice, ttsVoiceOptionsFor } from './ttsVoiceOptions';

export type { TaskModelSelection, TaskModelProviderScope } from './taskModelSelectState';

interface TaskModelSelectProps {
  task: ModelTask;
  /** Extra capability the model must carry (e.g. `['vision_input']`). */
  traits?: ModelTrait[];
  value: TaskModelSelection | null;
  /** Fired only with a complete, live selection. */
  onChange: (next: TaskModelSelection) => void;
  scope?: TaskModelProviderScope;
  /** Render the third (voice) select — the speech-synthesis variant. */
  withVoice?: boolean;
  /** Candidate voice ids; the field stays free text regardless. */
  voiceOptions?: readonly string[];
  size?: 'mini' | 'small' | 'default';
  disabled?: boolean;
  /** Suppress the inline warning line (the caller renders its own copy). */
  hideHint?: boolean;
  /** Receive the resolved warning copy when the caller positions it elsewhere. */
  onHintChange?: (hint: string) => void;
  /** Copy shown when the catalog has no model for this task at all. */
  emptyHint?: string;
  /** Optional copy for the model field; provider keeps its task-aware default. */
  placeholder?: string;
}

/**
 * The shared "pick a model for this task" control: provider + model, plus a
 * voice id for speech synthesis.
 *
 * Membership comes from `useModelsForTask`, which reads the capability nested
 * under each provider model — no name heuristics or second profile request.
 * Every judgement about the SAVED reference lives in
 * `taskModelSelectState`, so a stale provider and a stale model are rendered as
 * explicit disabled "(unavailable)" options rather than silently blanked: the
 * saved value stays visible and the user is told to re-pick.
 */
const TaskModelSelect: React.FC<TaskModelSelectProps> = ({
  task,
  traits,
  value,
  onChange,
  scope = 'task',
  withVoice = false,
  voiceOptions,
  size = 'mini',
  disabled = false,
  hideHint = false,
  onHintChange,
  emptyHint,
  placeholder,
}) => {
  const { t } = useTranslation();
  const { groups, isLoading } = useModelsForTask(task, traits);
  const { data: rawProviders } = useProvidersQuery();
  const providerLabel = useModelSelectorProviderLabel();
  const [draftProviderId, setDraftProviderId] = useState<ProviderId | null>(null);

  useEffect(() => {
    setDraftProviderId(value?.provider_id ?? null);
  }, [value?.provider_id]);

  const enabledProviders = useMemo(
    () => (rawProviders ?? []).filter((p) => p.enabled !== false),
    [rawProviders]
  );

  const state = taskModelSelectState({
    groups,
    enabledProviders,
    scope,
    value,
    draftProviderId,
    isLoading,
  });

  const providerId = draftProviderId;
  const selectedModel = value?.provider_id === providerId ? value.model : null;
  const selectedProvider = state.providers.find((provider) => provider.id === providerId);
  const speechSynthesisProtocol = selectedModel
    ? capabilityOf(selectedProvider, selectedModel, 'speech_synthesis')?.protocol
    : undefined;
  const modelIdIsVoice = ttsUsesModelIdAsVoice(speechSynthesisProtocol);
  const voices = voiceOptions ?? ttsVoiceOptionsFor(speechSynthesisProtocol, selectedModel ?? undefined);
  const selectedVoice = value?.provider_id === providerId ? (value.voice ?? null) : null;

  const displayModelLabel = (model: string): string => {
    const row = selectedProvider?.models.find((candidate) => candidate.model === model);
    return modelDisplayLabel(model, row?.display_name);
  };

  const hint =
    !state.anyModel && !isLoading
      ? (emptyHint ?? t('settings.taskModel.emptyHint'))
      : state.providerStale
        ? t('settings.taskModel.staleHint', { model: providerId ?? '' })
        : state.modelStale && selectedModel
          ? t('settings.taskModel.staleHint', { model: selectedModel })
          : '';

  useEffect(() => {
    onHintChange?.(hint);
  }, [hint, onHintChange]);

  return (
    <div className='flex min-w-0 flex-col items-end gap-4px'>
      <div className='flex min-w-0 flex-wrap items-center justify-end gap-6px'>
        <NomiSelect
          size={size}
          contentFit
          contentMaxWidth={220}
          disabled={disabled}
          placeholder={t('settings.taskModel.providerPlaceholder')}
          value={providerId ?? undefined}
          onChange={(next: ProviderId) => setDraftProviderId(next)}
        >
          {state.providerStale && providerId && (
            <NomiSelect.Option key={providerId} value={providerId} disabled>
              {t('settings.taskModel.unavailableOption', { model: providerId })}
            </NomiSelect.Option>
          )}
          {state.providers.map((p) => (
            <NomiSelect.Option key={p.id} value={p.id}>
              {providerLabel(p)}
            </NomiSelect.Option>
          ))}
        </NomiSelect>
        <NomiSelect
          size={size}
          contentFit
          contentMaxWidth={280}
          disabled={disabled || providerId == null || state.providerStale}
          placeholder={placeholder ?? t('settings.taskModel.modelPlaceholder')}
          value={selectedModel ?? undefined}
          onChange={(model: string) => {
            if (!providerId) return;
            const nextSpeechSynthesisProtocol = capabilityOf(
              selectedProvider,
              model,
              'speech_synthesis'
            )?.protocol;
            onChange({
              provider_id: providerId,
              model,
              // Re-picking the model must keep a voice already chosen for this
              // provider; only a provider switch resets it.
              voice:
                !ttsUsesModelIdAsVoice(nextSpeechSynthesisProtocol) &&
                value?.provider_id === providerId
                  ? value.voice
                  : null,
            });
          }}
        >
          {state.modelStale && selectedModel && (
            <NomiSelect.Option key={selectedModel} value={selectedModel} disabled>
              {displayModelLabel(selectedModel)} · {t('settings.taskModel.unavailableOption', { model: selectedModel })}
            </NomiSelect.Option>
          )}
          {state.models.map((m) => (
            <NomiSelect.Option key={m} value={m}>
              {displayModelLabel(m)}
            </NomiSelect.Option>
          ))}
        </NomiSelect>
        {withVoice && !modelIdIsVoice && (
          <NomiSelect
            size={size}
            contentFit
            contentMaxWidth={200}
            showSearch
            allowCreate
            disabled={disabled || selectedModel == null}
            placeholder={t('settings.taskModel.voicePlaceholder')}
            value={selectedVoice ?? undefined}
            onChange={(voice: string) => {
              if (!providerId || !selectedModel) return;
              onChange({ provider_id: providerId, model: selectedModel, voice: voice || null });
            }}
          >
            {voices.map((voice) => (
              <NomiSelect.Option key={voice} value={voice}>
                {voice}
              </NomiSelect.Option>
            ))}
          </NomiSelect>
        )}
      </div>
      {!hideHint && hint && <span className='text-11px leading-tight text-warning-6'>{hint}</span>}
    </div>
  );
};

export default TaskModelSelect;
