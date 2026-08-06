/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Right } from '@icon-park/react';
import type { ICompanionStatus } from '@/common/adapter/ipcBridge';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import CompanionModelControl from '@renderer/pages/nomi/CompanionModelControl';
import type { CompanionHandle } from '../../types';
import RowAction from './RowAction';

interface ModelsSectionProps {
  companion: CompanionHandle;
  status: ICompanionStatus;
  companionName: string;
}

/**
 * 模型配置 — the brains. Only the chat model is genuinely per-companion in this
 * build; ASR is a global config value (`tools.speechToText`) and TTS / VAD /
 * vision have no per-companion setting at all. So instead of a wall of disabled
 * selects (the clutter this redesign removes) there is ONE row that says so and
 * links to the app-level model settings.
 */
const ModelsSection: React.FC<ModelsSectionProps> = ({ companion, status, companionName }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();

  return (
    <NomiSettingSection
      title={t('nomi.overview.modelSection', { defaultValue: '模型配置' })}
      description={t('nomi.overview.modelSectionHint', { defaultValue: '决定它用哪个模型思考与回应' })}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.overview.mainChatModel', { defaultValue: '主对话模型' })}
          description={
            status.model_configured
              ? t('nomi.chat.modelConfigHint', {
                  defaultValue: '该伙伴的对话模型，全局生效（本地对话与远程连接），切换后所有会话跟随',
                })
              : t('nomi.overview.modelMissing', {
                  defaultValue: '还没有为 {{companionName}} 配置聊天模型，它暂时无法学习和聊天。',
                  companionName,
                })
          }
          style={status.model_configured ? undefined : { background: 'rgb(var(--warning-1))' }}
          controls={<CompanionModelControl companion={companion} showLabel={false} />}
        />

        <NomiSettingRow
          title={t('nomi.overview.voicePerception', { defaultValue: '语音与感知' })}
          description={t('nomi.overview.voicePerceptionHint', {
            defaultValue: '语音识别、语音合成与视觉模型是应用级设置，所有伙伴共用，在「模型管理」里统一配置',
          })}
          controls={
            <RowAction onClick={() => navigate('/models')}>
              {t('nomi.overview.goModelSettings', { defaultValue: '前往设置' })}
              <Right theme='outline' size='14' fill='currentColor' strokeWidth={3} />
            </RowAction>
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default ModelsSection;
