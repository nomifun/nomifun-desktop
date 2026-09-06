/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Input } from '@arco-design/web-react';
import type { ICompanionProfile } from '@/common/adapter/ipcBridge';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import type { CompanionHandle } from '../../types';
import { useDebouncedText } from './useDebouncedText';

interface PersonaSectionProps {
  profile: ICompanionProfile;
  patchCompanion: CompanionHandle['patchCompanion'];
}

/**
 * Companion persona settings are local tone/notes controls. AgentPreset
 * authoring and reusable Agent configuration belong to the Agent Workbench,
 * so this surface has no preset picker or apply action.
 */
const PersonaSection: React.FC<PersonaSectionProps> = ({ profile, patchCompanion }) => {
  const { t } = useTranslation();
  const companionName = profile.name;

  const [customDraft, onCustomChange] = useDebouncedText(profile.persona.custom ?? '', (custom) => {
    if (custom === (profile.persona.custom ?? '')) return;
    void patchCompanion({ persona: { custom } }).catch(() => undefined);
  });

  return (
    <NomiSettingSection
      title={t('nomi.overview.personaSection', { defaultValue: 'Persona' })}
      description={t('nomi.overview.personaSectionHint', {
        defaultValue: 'Choose how this companion speaks in future conversations.',
      })}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.overview.personaTitle', { defaultValue: 'Tone and notes' })}
          description={t('nomi.settings.personaHint', {
            defaultValue: 'Set the tone and optional instructions for {{companionName}}.',
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
                {t('nomi.settings.personaLively', { defaultValue: 'Lively' })}
              </NomiSelect.Option>
              <NomiSelect.Option value='calm'>
                {t('nomi.settings.personaCalm', { defaultValue: 'Calm' })}
              </NomiSelect.Option>
              <NomiSelect.Option value='sassy'>
                {t('nomi.settings.personaSassy', { defaultValue: 'Sassy' })}
              </NomiSelect.Option>
            </NomiSelect>
          }
          footer={
            <Input.TextArea
              autoSize={{ minRows: 1, maxRows: 4 }}
              className='!bg-[var(--color-bg-1)] !border-[var(--color-border-2)] !rd-8px !px-10px !py-7px !leading-20px'
              placeholder={t('nomi.settings.personaCustomPlaceholder', {
                defaultValue: 'Optional persona notes, for example: call me team lead.',
              })}
              value={customDraft}
              onChange={onCustomChange}
            />
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default PersonaSection;
