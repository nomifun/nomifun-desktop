/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Spin, Switch } from '@arco-design/web-react';
import type { ProviderId } from '@/common/types/ids';
import NomiInputNumber from '@/renderer/components/base/NomiInputNumber';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import { useModelsForTask } from '@renderer/hooks/agent/useModelsForTask';
import type { useCompanionShared } from '../useNomi';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';

const COMPANION_SWITCH_PROPS = { size: 'small' as const, className: 'compact-dark-switch' };

interface Props {
  shared: ReturnType<typeof useCompanionShared>;
  collectionSection?: React.ReactNode;
}

const LearnTab: React.FC<Props> = ({ shared, collectionSection }) => {
  const { t } = useTranslation();
  const { sharedConfig, patchSharedConfig } = shared;
  // 学习模型做对话补全 —— 供应商/模型清单来自统一 chat catalog（后端 resolve）。
  const { groups: chatGroups } = useModelsForTask('chat');
  const providers = useMemo(() => chatGroups.map((group) => group.provider), [chatGroups]);
  const providerLabel = useModelSelectorProviderLabel();
  const [draftProviderId, setDraftProviderId] = useState<ProviderId | null>(null);

  useEffect(() => {
    setDraftProviderId(sharedConfig?.learn.model?.provider_id ?? null);
  }, [sharedConfig?.learn.model?.provider_id]);

  const currentProvider = useMemo(
    () => providers.find((p) => p.id === draftProviderId),
    [draftProviderId, providers]
  );
  const currentProviderModels = useMemo(
    () => chatGroups.find((group) => group.provider.id === draftProviderId)?.models ?? [],
    [chatGroups, draftProviderId]
  );

  if (!sharedConfig) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  return (
    <div className='flex flex-col gap-22px py-8px'>
      <NomiSettingSection title={t('nomi.learn.sectionTitle')}>
        <NomiSettingList>
          <NomiSettingRow
            title={t('nomi.learn.enabled')}
            controls={
              <Switch
                {...COMPANION_SWITCH_PROPS}
                checked={sharedConfig.learn.enabled}
                onChange={(checked) => void patchSharedConfig({ learn: { enabled: checked } })}
              />
            }
          />
          <NomiSettingRow
            title={t('nomi.learn.interval')}
            controls={
              <NomiInputNumber
                contentFit
                min={5}
                max={1440}
                value={sharedConfig.learn.interval_minutes}
                onChange={(v) => void patchSharedConfig({ learn: { interval_minutes: Number(v) || 60 } })}
                suffix={t('nomi.learn.minutes')}
              />
            }
          />
          <NomiSettingRow
            title={t('nomi.learn.model')}
            description={t('nomi.learn.modelHint')}
            controls={
              <>
              <NomiSelect
                contentFit
                contentMaxWidth={240}
                placeholder={t('nomi.settings.providerPlaceholder')}
                value={draftProviderId ?? undefined}
                onChange={(provider_id: ProviderId) => setDraftProviderId(provider_id)}
              >
                {providers.map((p) => (
                  <NomiSelect.Option key={p.id} value={p.id}>
                    {providerLabel(p)}
                  </NomiSelect.Option>
                ))}
              </NomiSelect>
              <NomiSelect
                contentFit
                contentMaxWidth={300}
                placeholder={t('nomi.settings.modelPlaceholder')}
                value={
                  sharedConfig.learn.model?.provider_id === draftProviderId
                    ? sharedConfig.learn.model.model
                    : undefined
                }
                disabled={!currentProvider}
                onChange={(model: string) => {
                  if (draftProviderId) {
                    void patchSharedConfig({ learn: { model: { provider_id: draftProviderId, model } } });
                  }
                }}
              >
                {(currentProvider ? currentProviderModels : []).map((m) => (
                  <NomiSelect.Option key={m} value={m}>
                    {m}
                  </NomiSelect.Option>
                ))}
              </NomiSelect>
              </>
            }
          />
        </NomiSettingList>
      </NomiSettingSection>

      {collectionSection}

      <NomiSettingSection
        title={t('nomi.evolve.sectionTitle')}
        description={t('nomi.evolve.hint', {
          defaultValue: '从你重复的多步操作里自动沉淀技能，复用上面的学习模型。',
        })}
      >
        <NomiSettingList>
          <NomiSettingRow
            title={t('nomi.evolve.enabled')}
            controls={
              <Switch
                {...COMPANION_SWITCH_PROPS}
                checked={sharedConfig.evolve.enabled}
                onChange={(checked) => void patchSharedConfig({ evolve: { enabled: checked } })}
              />
            }
          />
          <NomiSettingRow
            title={t('nomi.evolve.autoActivate')}
            description={t('nomi.evolve.autoActivateHint')}
            controls={
              <Switch
                {...COMPANION_SWITCH_PROPS}
                checked={sharedConfig.evolve.auto_activate}
                onChange={(checked) => void patchSharedConfig({ evolve: { auto_activate: checked } })}
              />
            }
          />
          <NomiSettingRow
            title={t('nomi.evolve.interval')}
            controls={
              <NomiInputNumber
                contentFit
                min={5}
                max={1440}
                value={sharedConfig.evolve.interval_minutes}
                onChange={(v) => void patchSharedConfig({ evolve: { interval_minutes: Number(v) || 30 } })}
                suffix={t('nomi.learn.minutes')}
              />
            }
          />
          <NomiSettingRow
            title={t('nomi.evolve.minSessions')}
            controls={
              <NomiInputNumber
                contentFit
                min={2}
                max={20}
                value={sharedConfig.evolve.min_distinct_sessions}
                onChange={(v) => void patchSharedConfig({ evolve: { min_distinct_sessions: Number(v) || 2 } })}
              />
            }
          />
        </NomiSettingList>
      </NomiSettingSection>

      <NomiSettingSection title={t('nomi.collaboration.title')} description={t('nomi.collaboration.hint')}>
        <NomiSettingList>
          <NomiSettingRow
            title={t('nomi.collaboration.enabled')}
            controls={
              <Switch
                {...COMPANION_SWITCH_PROPS}
                checked={sharedConfig.smart_collaboration ?? false}
                onChange={(checked) => void patchSharedConfig({ smart_collaboration: checked })}
              />
            }
          />
        </NomiSettingList>
      </NomiSettingSection>

      <NomiSettingSection title={t('nomi.archive.title')} description={t('nomi.archive.hint')}>
        <NomiSettingList>
          <NomiSettingRow
            title={t('nomi.archive.enabled')}
            controls={
              <Switch
                {...COMPANION_SWITCH_PROPS}
                checked={sharedConfig.archive?.enabled ?? false}
                onChange={(checked) => void patchSharedConfig({ archive: { enabled: checked } })}
              />
            }
          />
          <NomiSettingRow
            title={t('nomi.archive.idleMinutes')}
            controls={
              <NomiInputNumber
                contentFit
                min={5}
                max={1440}
                value={sharedConfig.archive?.idle_minutes ?? 30}
                onChange={(v) => void patchSharedConfig({ archive: { idle_minutes: Number(v) || 30 } })}
                suffix={t('nomi.learn.minutes')}
              />
            }
          />
        </NomiSettingList>
      </NomiSettingSection>
    </div>
  );
};

export default LearnTab;
