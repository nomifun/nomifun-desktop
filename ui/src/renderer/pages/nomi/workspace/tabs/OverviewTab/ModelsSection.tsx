/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import type { ICompanionStatus } from '@/common/adapter/ipcBridge';
import NomiInputNumber from '@/renderer/components/base/NomiInputNumber';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import CompanionModelControl from '@renderer/pages/nomi/CompanionModelControl';
import type { CompanionHandle } from '../../types';

interface ModelsSectionProps {
  companion: CompanionHandle;
  status: ICompanionStatus;
  companionName: string;
}

/**
 * 模型配置 — the brains, now five kinds of slot instead of one.
 *
 * Every slot except VAD is a catalog reference and therefore renders through the
 * shared `TaskModelSelect`; VAD is local (Silero, no Provider, no credential)
 * and so is two numeric parameters. An unset slot is NOT an error: each row says
 * what it falls back to, which is why the old "voice & perception are app-level,
 * go configure them elsewhere" redirect row is gone — the controls are here now.
 */
const ModelsSection: React.FC<ModelsSectionProps> = ({ companion, status, companionName }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;

  if (!profile) return null;

  const vad = profile.voice.vad;

  return (
    <NomiSettingSection
      title={t('nomi.overview.modelSection')}
      description={t('nomi.overview.modelSectionHint')}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.overview.mainChatModel')}
          description={
            status.model_configured
              ? t('nomi.chat.modelConfigHint')
              : t('nomi.overview.modelMissing', { companionName })
          }
          style={status.model_configured ? undefined : { background: 'rgb(var(--warning-1))' }}
          controls={<CompanionModelControl companion={companion} showLabel={false} />}
        />

        <NomiSettingRow
          title={t('nomi.overview.fallbackChatModel')}
          description={
            profile.fallback_model
              ? t('nomi.overview.fallbackChatModelHint')
              : t('nomi.overview.fallbackChatUnset')
          }
          controls={
            <TaskModelSelect
              task='chat'
              value={profile.fallback_model}
              onChange={({ provider_id, model }) =>
                void patchCompanion({ fallback_model: { provider_id, model } })
              }
            />
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.vadSlot')}
          description={t('nomi.overview.vadSlotHint')}
          controls={
            <>
              <span className='text-12px text-t-tertiary shrink-0'>
                {t('nomi.overview.vadSensitivity')}
              </span>
              <NomiInputNumber
                size='mini'
                contentFit
                min={0}
                max={1}
                step={0.05}
                precision={2}
                value={vad.sensitivity}
                onChange={(sensitivity) => {
                  if (typeof sensitivity !== 'number') return;
                  void patchCompanion({ voice: { vad: { sensitivity } } });
                }}
              />
              <span className='text-12px text-t-tertiary shrink-0'>
                {t('nomi.overview.vadMinSilence')}
              </span>
              <NomiInputNumber
                size='mini'
                contentFit
                min={200}
                max={3000}
                step={50}
                value={vad.min_silence_ms}
                onChange={(min_silence_ms) => {
                  if (typeof min_silence_ms !== 'number') return;
                  void patchCompanion({ voice: { vad: { min_silence_ms } } });
                }}
              />
            </>
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.asrSlot')}
          description={
            profile.voice.asr ? t('nomi.overview.asrSlotHint') : t('nomi.overview.asrFallback')
          }
          controls={
            <TaskModelSelect
              task='speech_recognition'
              value={profile.voice.asr}
              onChange={({ provider_id, model }) =>
                void patchCompanion({ voice: { asr: { provider_id, model } } })
              }
            />
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.visionSlot')}
          description={
            profile.vision_model
              ? t('nomi.overview.visionSlotHint')
              : t('nomi.overview.visionFallback')
          }
          controls={
            <TaskModelSelect
              task='chat'
              traits={['vision_input']}
              value={profile.vision_model}
              onChange={({ provider_id, model }) =>
                void patchCompanion({ vision_model: { provider_id, model } })
              }
            />
          }
        />

        <NomiSettingRow
          title={t('nomi.overview.ttsSlot')}
          description={
            profile.voice.tts ? t('nomi.overview.ttsSlotHint') : t('nomi.overview.ttsFallback')
          }
          controls={
            <TaskModelSelect
              task='speech_synthesis'
              withVoice
              value={profile.voice.tts}
              onChange={({ provider_id, model, voice }) =>
                void patchCompanion({ voice: { tts: { provider_id, model, voice: voice ?? null } } })
              }
            />
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default ModelsSection;
