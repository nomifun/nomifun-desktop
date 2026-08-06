/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Input, Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ICompanionProfile } from '@/common/adapter/ipcBridge';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import PresetApplyControl from '@/renderer/components/preset/PresetApplyControl';
import type { CompanionHandle } from '../../types';
import { useDebouncedText } from './useDebouncedText';

interface PersonaSectionProps {
  profile: ICompanionProfile;
  patchCompanion: CompanionHandle['patchCompanion'];
  /** Re-read the profile after the backend applied a preset snapshot. */
  refresh: CompanionHandle['refresh'];
}

/**
 * 伙伴设定 — how this companion talks: a tone preset plus free-text notes, and the
 * one-click reuse of a saved preset. Both rows write the same idea (“who it is”),
 * so they live in one list.
 */
const PersonaSection: React.FC<PersonaSectionProps> = ({ profile, patchCompanion, refresh }) => {
  const { t } = useTranslation();
  const companionName = profile.name;

  const [customDraft, onCustomChange] = useDebouncedText(profile.persona.custom ?? '', (custom) => {
    if (custom === (profile.persona.custom ?? '')) return;
    void patchCompanion({ persona: { custom } }).catch((e) => Message.error(String(e)));
  });

  return (
    <NomiSettingSection
      title={t('nomi.overview.personaSection', { defaultValue: '伙伴设定' })}
      description={t('nomi.overview.personaSectionHint', { defaultValue: '它是谁、怎么说话，都会写进每次对话的开场' })}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.overview.personaTitle', { defaultValue: '角色介绍' })}
          description={t('nomi.settings.personaHint', {
            defaultValue: '决定 {{companionName}} 说话的性格与语气',
            companionName,
          })}
          controls={
            <NomiSelect
              contentFit
              contentMaxWidth={260}
              value={profile.persona.preset}
              onChange={(preset: string) => void patchCompanion({ persona: { preset } })}
            >
              <NomiSelect.Option value='lively'>
                {t('nomi.settings.personaLively', { defaultValue: '活泼' })}
              </NomiSelect.Option>
              <NomiSelect.Option value='calm'>{t('nomi.settings.personaCalm', { defaultValue: '沉稳' })}</NomiSelect.Option>
              <NomiSelect.Option value='sassy'>
                {t('nomi.settings.personaSassy', { defaultValue: '小毒舌' })}
              </NomiSelect.Option>
            </NomiSelect>
          }
          footer={
            <Input.TextArea
              autoSize={{ minRows: 1, maxRows: 4 }}
              className='!bg-[var(--color-bg-1)] !border-[var(--color-border-2)] !rd-8px !px-10px !py-7px !leading-20px'
              placeholder={t('nomi.settings.personaCustomPlaceholder', {
                defaultValue: '补充人格设定（可选），例如：叫我「队长」',
              })}
              value={customDraft}
              onChange={onCustomChange}
            />
          }
        />

        <NomiSettingRow
          title={t('nomi.settings.preset', { defaultValue: '复用设定' })}
          description={t('nomi.settings.presetHint', {
            defaultValue: '一键应用已保存的 Agent、模型、Skill 与知识范围配置。',
          })}
          controls={
            <PresetApplyControl
              compact
              target='companion'
              appliedPreset={profile.applied_preset}
              onApply={async (presetId, locale) => {
                await ipcBridge.companion.applyPreset.invoke({
                  companion_id: profile.companion_id,
                  preset_id: presetId,
                  locale,
                });
                await refresh();
              }}
            />
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default PersonaSection;
