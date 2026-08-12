/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import type { SpeechToTextConfig } from '@/common/types/provider/speech';

export const SPEECH_TO_TEXT_CONFIG_KEY = 'tools.speechToText' as const;
export const SPEECH_TO_TEXT_CONFIG_CHANGED_EVENT = 'nomifun:speech-to-text-config-changed';

export const DEFAULT_SPEECH_TO_TEXT_CONFIG: SpeechToTextConfig = {
  enabled: false,
  language: '',
};

export const normalizeSpeechToTextConfig = (config?: SpeechToTextConfig): SpeechToTextConfig => ({
  enabled: config?.enabled ?? false,
  language: config?.language ?? '',
  ...(config?.autoSend === undefined ? {} : { autoSend: config.autoSend }),
  ...(config?.provider_id === undefined ? {} : { provider_id: config.provider_id }),
  ...(config?.model === undefined ? {} : { model: config.model }),
});

export const getSpeechToTextConfig = (): SpeechToTextConfig =>
  normalizeSpeechToTextConfig(configService.get(SPEECH_TO_TEXT_CONFIG_KEY));

export const saveSpeechToTextConfig = async (config: SpeechToTextConfig): Promise<void> => {
  const normalized = normalizeSpeechToTextConfig(config);
  try {
    await configService.set(SPEECH_TO_TEXT_CONFIG_KEY, normalized);
  } catch (error) {
    await configService.reload();
    throw error;
  } finally {
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent(SPEECH_TO_TEXT_CONFIG_CHANGED_EVENT));
    }
  }
};
