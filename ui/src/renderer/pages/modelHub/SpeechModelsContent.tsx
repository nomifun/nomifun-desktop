/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import SpeechToTextContent from './SpeechToTextContent';

/**
 * 语音区宿主：语音识别（ASR）、语音合成（TTS）与语音活动检测（VAD）三块。
 * 每块自己拉自己的配置，本文件只负责纵向排布。
 */
const SpeechModelsContent: React.FC = () => (
  <div className='flex flex-col gap-14px'>
    <SpeechToTextContent />
  </div>
);

export default SpeechModelsContent;
