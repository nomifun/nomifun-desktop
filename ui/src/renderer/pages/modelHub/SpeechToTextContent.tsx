/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Form, Input, Switch } from '@arco-design/web-react';
import { HeadsetOne, LinkCloud, Radar } from '@icon-park/react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { configService } from '@/common/config/configService';
import type { SpeechToTextConfig } from '@/common/types/provider/speech';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import {
  DEFAULT_SPEECH_TO_TEXT_CONFIG,
  getSpeechToTextConfig,
  hasLegacyEmbeddedSpeechBlocks,
  normalizeSpeechToTextConfig,
  saveSpeechToTextConfig,
  SPEECH_TO_TEXT_CONFIG_CHANGED_EVENT,
  SPEECH_TO_TEXT_CONFIG_KEY,
} from '@/renderer/services/speechToTextConfig';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';

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

  useEffect(() => {
    const syncConfig = () => setConfig(getSpeechToTextConfig());
    syncConfig();
    window.addEventListener(SPEECH_TO_TEXT_CONFIG_CHANGED_EVENT, syncConfig);
    return () => window.removeEventListener(SPEECH_TO_TEXT_CONFIG_CHANGED_EVENT, syncConfig);
  }, []);

  // One-time migration: a config still carrying a retired embedded-credential
  // block is rewritten in the catalog shape the moment this page is opened. The
  // backend has refused those blocks since the catalog migration, so leaving
  // them on disk only keeps a dead API key around.
  useEffect(() => {
    const stored = getSpeechToTextConfig();
    if (!hasLegacyEmbeddedSpeechBlocks(configService.get(SPEECH_TO_TEXT_CONFIG_KEY))) return;
    void saveSpeechToTextConfig(stored).catch((error) => {
      console.error('Failed to migrate the legacy speech-to-text config:', error);
    });
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
      <div className='flex min-h-0 flex-col rd-16px bg-2 px-24px py-16px'>
        {messageContext}
        <header className='flex items-center gap-9px border-b border-b-solid border-[var(--color-border-2)] pb-14px'>
          <span className='size-30px shrink-0 flex items-center justify-center rd-9px bg-primary-1 text-primary-6'>
            <HeadsetOne theme='outline' size='18' strokeWidth={3} />
          </span>
          <div className='min-w-0'>
            <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>
              {t('settings.modelHub.speech.asrTitle')}
            </h2>
            <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>
              {t('settings.modelHub.speech.asrSubtitle')}
            </p>
          </div>
        </header>

        <Form layout='vertical' className='mt-18px'>
          <Form.Item label={t('settings.modelHub.speech.source')}>
            <TaskModelSelect
              task='speech_recognition'
              size='default'
              value={
                config.provider_id && config.model
                  ? { provider_id: config.provider_id, model: config.model }
                  : null
              }
              emptyHint={t('settings.modelHub.speech.noSources')}
              onChange={({ provider_id, model }) =>
                persist({ ...config, enabled: true, provider: 'openai', provider_id, model })
              }
            />
          </Form.Item>
          <Form.Item label={t('settings.modelHub.speech.defaultLanguage')}>
            <Input
              value={config.language}
              placeholder={t('settings.modelHub.speech.languagePlaceholder')}
              onBlur={() => persist(config)}
              onChange={(language) => setConfig((current) => ({ ...current, language }))}
            />
          </Form.Item>
          <Form.Item label={t('settings.modelHub.speech.enabled')}>
            <Switch
              checked={config.enabled && selected}
              disabled={!selected}
              onChange={(enabled) => persist({ ...config, enabled })}
            />
          </Form.Item>
        </Form>

        <div className='mt-6px flex items-center gap-8px flex-wrap'>
          <Button
            type='text'
            size='small'
            icon={<LinkCloud theme='outline' size='14' />}
            onClick={() => navigate('/models?section=models')}
          >
            {t('settings.modelHub.speech.manageProviders')}
          </Button>
        </div>
      </div>

      {/* 本机 VAD：没有可选模型，只陈述引擎与默认值。 */}
      <div className='flex min-h-0 flex-col rd-16px bg-2 px-24px py-16px'>
        <header className='flex items-center gap-9px pb-4px'>
          <span className='size-30px shrink-0 flex items-center justify-center rd-9px bg-primary-1 text-primary-6'>
            <Radar theme='outline' size='18' strokeWidth={3} />
          </span>
          <div className='min-w-0'>
            <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>
              {t('settings.modelHub.speech.vadTitle')}
            </h2>
            <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>
              {t('settings.modelHub.speech.vadBuiltin')}
            </p>
          </div>
        </header>
        <p className='m-0 mt-8px text-12px leading-18px text-t-secondary'>
          {t('settings.modelHub.speech.vadBuiltinHint', { sensitivity: '0.5', silence: 700 })}
        </p>
      </div>
    </div>
  );
};

export default SpeechToTextContent;
