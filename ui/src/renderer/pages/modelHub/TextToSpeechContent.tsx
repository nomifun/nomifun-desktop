/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Form } from '@arco-design/web-react';
import { Voice } from '@icon-park/react';
import type { TextToSpeechConfig } from '@/common/types/provider/speech';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import {
  getTextToSpeechConfig,
  saveTextToSpeechConfig,
  TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT,
} from '@/renderer/services/textToSpeechConfig';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import ModalityModelsPanel from './ModalityModelsPanel';

/**
 * TTS 首次获得配置面：全局默认的「语音合成模型 + 音色」。
 *
 * Every companion whose `voice.tts` slot is empty speaks with this. Clearing it
 * deletes the preference key outright (see `saveTextToSpeechConfig`), because
 * the backend registers the key as a required Provider reference and would
 * refuse a half-empty object.
 */
const TextToSpeechContent: React.FC = () => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage({ maxCount: 2 });
  const [config, setConfig] = useState<TextToSpeechConfig | null>(null);

  useEffect(() => {
    const sync = () => setConfig(getTextToSpeechConfig() ?? null);
    sync();
    window.addEventListener(TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT, sync);
    return () => window.removeEventListener(TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT, sync);
  }, []);

  const persist = useCallback(
    (next: TextToSpeechConfig | null) => {
      setConfig(next);
      void saveTextToSpeechConfig(next)
        .then(() => message.success(t('settings.modelHub.speech.ttsSaved')))
        .catch((error: unknown) => {
          setConfig(getTextToSpeechConfig() ?? null);
          message.error(
            error instanceof Error ? error.message : t('settings.modelHub.speech.ttsSaveFailed')
          );
        });
    },
    [message, t]
  );

  return (
    <div className='flex flex-col gap-14px'>
      <ModalityModelsPanel
        modality='tts'
        icon={<Voice theme='outline' size='18' strokeWidth={3} />}
        titleKey='settings.modelHub.modality.ttsTitle'
        subtitleKey='settings.modelHub.modality.ttsSubtitle'
      />
      <div className='flex min-h-0 flex-col rd-16px bg-2 px-24px py-16px'>
        {messageContext}
      <header className='flex items-center gap-9px border-b border-b-solid border-[var(--color-border-2)] pb-14px'>
        <span className='size-30px shrink-0 flex items-center justify-center rd-9px bg-primary-1 text-primary-6'>
          <Voice theme='outline' size='18' strokeWidth={3} />
        </span>
        <div className='min-w-0'>
          <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>
            {t('settings.modelHub.speech.ttsTitle')}
          </h2>
          <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>
            {t('settings.modelHub.speech.ttsSubtitle')}
          </p>
        </div>
      </header>

      <Form layout='vertical' className='mt-18px'>
        <Form.Item
          label={t('settings.modelHub.speech.ttsSource')}
          extra={t('settings.taskModel.voiceFreeTextHint')}
        >
          <TaskModelSelect
            task='speech_synthesis'
            size='default'
            withVoice
            value={config}
            emptyHint={t('settings.modelHub.speech.ttsNoSources')}
            onChange={({ provider_id, model, voice }) =>
              persist({ provider_id, model, voice: voice ?? null })
            }
          />
        </Form.Item>
      </Form>

        {config && (
          <div className='flex items-center gap-8px'>
            <Button size='small' onClick={() => persist(null)}>
              {t('settings.modelHub.speech.ttsClear')}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
};

export default TextToSpeechContent;
