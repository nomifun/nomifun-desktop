/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Switch } from '@arco-design/web-react';
import { LinkCloud } from '@icon-park/react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import type { SpeechToTextConfig } from '@/common/types/provider/speech';
import NomiInput from '@/renderer/components/base/NomiInput';
import { NomiSettingList, NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import {
  DEFAULT_SPEECH_TO_TEXT_CONFIG,
  getSpeechToTextConfig,
  normalizeSpeechToTextConfig,
  saveSpeechToTextConfig,
  SPEECH_TO_TEXT_CONFIG_CHANGED_EVENT,
} from '@/renderer/services/speechToTextConfig';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import ModalityModelsPanel from './ModalityModelsPanel';
import ModelHubPageHeader from './ModelHubPageHeader';

/**
 * 语音识别（ASR）分区：哪个目录里的模型负责把说话转成文字，外加本机 VAD 说明。
 *
 * Candidates come from the authoritative catalog resolution for
 * `speech_recognition` through the shared `TaskModelSelect` — no provider-row
 * name guessing, and no second copy of the "stale reference" rendering rules.
 *
 * VAD sits here rather than in its own section because it is not a model picker:
 * the engine is the bundled Silero ONNX graph running locally, and the gateway
 * recognises only `"silero"` (anything else falls back to its energy detector).
 * It decides when listening starts and stops, i.e. it is the front half of
 * recognition. The tunable knobs stay per companion, on that companion's 总览
 * page, because a pause length that suits one companion's owner suits nothing
 * else.
 */
const SpeechToTextContent: React.FC = () => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [message, messageContext] = useArcoMessage({ maxCount: 2 });
  const [config, setConfig] = useState<SpeechToTextConfig>(DEFAULT_SPEECH_TO_TEXT_CONFIG);
  const [sourceHint, setSourceHint] = useState('');

  useEffect(() => {
    const syncConfig = () => setConfig(getSpeechToTextConfig());
    syncConfig();
    window.addEventListener(SPEECH_TO_TEXT_CONFIG_CHANGED_EVENT, syncConfig);
    return () => window.removeEventListener(SPEECH_TO_TEXT_CONFIG_CHANGED_EVENT, syncConfig);
  }, []);

  const persist = useCallback(
    (next: SpeechToTextConfig) => {
      const normalized = normalizeSpeechToTextConfig(next);
      setConfig(normalized);
      void saveSpeechToTextConfig(normalized).catch((error) => {
        console.error('Failed to save speech-to-text config:', error);
        setConfig(getSpeechToTextConfig());
        message.error(error instanceof Error ? error.message : t('settings.saveModelConfigFailed'));
      });
    },
    [message, t]
  );

  const selected = Boolean(config.provider_id && config.model);

  return (
    <div className='flex flex-col gap-14px'>
      <ModalityModelsPanel
        modality='asr'
        titleKey='settings.modelHub.modality.asrTitle'
        subtitleKey='settings.modelHub.modality.asrSubtitle'
      />
      <section className='flex min-h-0 flex-col border-t border-t-solid border-[var(--color-border-2)] pt-16px'>
        {messageContext}
        <ModelHubPageHeader
          title={t('settings.modelHub.speech.asrTitle')}
          description={t('settings.modelHub.speech.asrSubtitle')}
          actions={
            <Button
              type='text'
              size='small'
              className='shrink-0'
              icon={<LinkCloud theme='outline' size='14' />}
              onClick={() => navigate('/models?section=models')}
            >
              {t('settings.modelHub.speech.manageProviders')}
            </Button>
          }
        />

        <NomiSettingList className='mt-16px'>
          <NomiSettingRow
            title={t('settings.modelHub.speech.source')}
            description={sourceHint || undefined}
            descriptionClassName='!text-warning-6'
            controls={
              <TaskModelSelect
                task='speech_recognition'
                size='mini'
                hideHint
                onHintChange={setSourceHint}
                value={
                  config.provider_id && config.model
                    ? { provider_id: config.provider_id, model: config.model }
                    : null
                }
                emptyHint={t('settings.modelHub.speech.noSources')}
                onChange={({ provider_id, model }) =>
                  persist({ ...config, enabled: true, provider_id, model })
                }
              />
            }
          />
          <NomiSettingRow
            title={t('settings.modelHub.speech.defaultLanguage')}
            controls={
              <NomiInput
                size='mini'
                contentFit
                contentMinWidth={120}
                contentMaxWidth={180}
                className='max-w-full'
                value={config.language}
                placeholder={t('settings.modelHub.speech.languagePlaceholder')}
                onBlur={() => persist(config)}
                onChange={(language) => setConfig((current) => ({ ...current, language }))}
              />
            }
          />
          <NomiSettingRow
            title={t('settings.modelHub.speech.enabled')}
            controls={
              <Switch
                size='small'
                className='compact-dark-switch shrink-0'
                checked={config.enabled && selected}
                disabled={!selected}
                onChange={(enabled) => persist({ ...config, enabled })}
              />
            }
          />
        </NomiSettingList>
      </section>

      {/* 本机 VAD：没有可选模型，只陈述引擎与默认值。 */}
      <section className='flex min-h-0 flex-col border-t border-t-solid border-[var(--color-border-2)] pt-16px'>
        <ModelHubPageHeader
          title={t('settings.modelHub.speech.vadTitle')}
          description={t('settings.modelHub.speech.vadBuiltin')}
        />
        <p className='m-0 mt-8px text-12px leading-18px text-t-secondary'>
          {t('settings.modelHub.speech.vadBuiltinHint', { sensitivity: '0.5', silence: 700 })}
        </p>
      </section>
    </div>
  );
};

export default SpeechToTextContent;
