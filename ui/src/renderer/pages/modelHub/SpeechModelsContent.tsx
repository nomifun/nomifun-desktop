/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Radar } from '@icon-park/react';
import SpeechToTextContent from './SpeechToTextContent';
import TextToSpeechContent from './TextToSpeechContent';

/**
 * 语音区宿主：语音识别（ASR）、语音合成（TTS）与语音活动检测（VAD）三块。
 *
 * VAD is not a model picker: the engine is the bundled Silero ONNX graph running
 * locally, and the gateway recognises only `"silero"` (anything else falls back
 * to its energy detector). So this section states the engine and its defaults;
 * the tunable knobs are per companion, on that companion's 总览 page, because a
 * pause length that suits one companion's owner suits nothing else.
 */
const SpeechModelsContent: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col gap-14px'>
      <SpeechToTextContent />
      <TextToSpeechContent />
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

export default SpeechModelsContent;
