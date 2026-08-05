/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { Attention } from '@icon-park/react';
import type { ProviderId } from '@/common/types/ids';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import { useModelsForTask } from '@renderer/hooks/agent/useModelsForTask';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import type { EvolutionConfigHandle, EvolutionLearnConfig } from './useEvolutionConfig';

interface Props {
  learn: EvolutionLearnConfig;
  patchLearn: EvolutionConfigHandle['patchLearn'];
  /** Learning (or skill generation) is on but no model is selected yet. */
  missing: boolean;
}

/**
 * Provider + model pair for the learning model. The catalog is the unified chat
 * resolution (`useModelsForTask('chat')`) — the same authority every other model
 * selector reads, so no frontend name heuristics. The saved reference is kept
 * verbatim: if its provider or model has since disappeared from the catalog the
 * row says so instead of silently blanking the setting.
 */
const LearningModelRow: React.FC<Props> = ({ learn, patchLearn, missing }) => {
  const { t } = useTranslation();
  const { groups, isLoading } = useModelsForTask('chat');
  const providerLabel = useModelSelectorProviderLabel();
  const providers = useMemo(() => groups.map((group) => group.provider), [groups]);
  const [draftProviderId, setDraftProviderId] = useState<ProviderId | null>(null);

  useEffect(() => {
    setDraftProviderId(learn.model?.provider_id ?? null);
  }, [learn.model?.provider_id]);

  const currentProvider = useMemo(
    () => providers.find((p) => p.id === draftProviderId),
    [draftProviderId, providers]
  );
  const currentModels = useMemo(
    () => groups.find((group) => group.provider.id === draftProviderId)?.models ?? [],
    [groups, draftProviderId]
  );

  const saved = learn.model;
  const stale =
    !isLoading &&
    saved != null &&
    !groups.some((group) => group.provider.id === saved.provider_id && group.models.includes(saved.model));
  // The saved provider vanished from the catalog (deleted, or it no longer offers a
  // chat model). Arco would render the bare provider id as the select's value, so
  // show it as an explicit disabled "(unavailable)" option instead — the same
  // treatment CompanionModelControl gives a stale provider.
  const providerStale = !isLoading && draftProviderId != null && !currentProvider;

  const commit = (provider_id: ProviderId, model: string) => {
    void patchLearn({ model: { provider_id, model } }).catch((e) => Message.error(String(e)));
  };

  return (
    <NomiSettingRow
      title={t('nomi.learn.model', { defaultValue: '学习模型' })}
      leading={
        missing ? (
          <Attention
            theme='filled'
            size='14'
            fill='currentColor'
            strokeWidth={3}
            className='line-height-0 shrink-0 text-danger-6'
          />
        ) : undefined
      }
      description={
        <>
          <div>
            {t('nomi.evolution.modelDesc', {
              defaultValue: '用来回顾记录、提炼记忆与技能的对话模型。',
            })}
          </div>
          {missing && (
            <div className='mt-2px text-danger-6'>
              {t('nomi.evolution.modelMissing', {
                defaultValue: '还没有选择模型，学习与技能生成都不会运行。',
              })}
            </div>
          )}
          {!missing && stale && saved && (
            <div className='mt-2px text-warning-6'>
              {t('nomi.evolution.modelStale', {
                defaultValue: '当前选择的 {{model}} 已不在可用模型清单里，学习会失败，请重新选择。',
                model: saved.model,
              })}
            </div>
          )}
        </>
      }
      controls={
        <>
          <NomiSelect
            contentFit
            contentMaxWidth={220}
            placeholder={t('nomi.settings.providerPlaceholder', { defaultValue: '选择服务商' })}
            value={draftProviderId ?? undefined}
            onChange={(provider_id: ProviderId) => setDraftProviderId(provider_id)}
          >
            {providerStale && draftProviderId && (
              <NomiSelect.Option key={draftProviderId} value={draftProviderId} disabled>
                {t('nomi.chat.modelUnavailableOption', { model: draftProviderId })}
              </NomiSelect.Option>
            )}
            {providers.map((p) => (
              <NomiSelect.Option key={p.id} value={p.id}>
                {providerLabel(p)}
              </NomiSelect.Option>
            ))}
          </NomiSelect>
          <NomiSelect
            contentFit
            contentMaxWidth={280}
            placeholder={t('nomi.settings.modelPlaceholder', { defaultValue: '选择模型' })}
            value={saved?.provider_id === draftProviderId ? saved.model : undefined}
            disabled={!currentProvider}
            onChange={(model: string) => {
              if (draftProviderId) commit(draftProviderId, model);
            }}
          >
            {(currentProvider ? currentModels : []).map((m) => (
              <NomiSelect.Option key={m} value={m}>
                {m}
              </NomiSelect.Option>
            ))}
          </NomiSelect>
        </>
      }
    />
  );
};

export default LearningModelRow;
