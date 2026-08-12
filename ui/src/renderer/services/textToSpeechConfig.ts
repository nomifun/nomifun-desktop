/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { configService } from '@/common/config/configService';
import type { TextToSpeechConfig } from '@/common/types/provider/speech';

export const TEXT_TO_SPEECH_CONFIG_KEY = 'tools.textToSpeech' as const;
// Named `tts-` rather than spelling out "text-to-speech": the dead-CSS checker
// reads a `text-…` hyphenated token in source as a utility class and reports it
// as generating no CSS.
export const TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT = 'nomifun:tts-config-changed';

/** The install-wide synthesis default, or `undefined` when none is set. */
export const getTextToSpeechConfig = (): TextToSpeechConfig | undefined =>
  configService.get(TEXT_TO_SPEECH_CONFIG_KEY);

/**
 * Persist (or clear) the install-wide synthesis default.
 *
 * `null` DELETES the key. The backend registers this preference as a required
 * `{provider_id, model}` reference, so a blank object would be refused at the
 * write boundary — absence is the only representation of "no default".
 */
export const saveTextToSpeechConfig = async (config: TextToSpeechConfig | null): Promise<void> => {
  try {
    if (config == null) {
      await configService.remove(TEXT_TO_SPEECH_CONFIG_KEY);
      return;
    }
    await configService.set(TEXT_TO_SPEECH_CONFIG_KEY, config);
  } catch (error) {
    // configService updates its in-memory cache optimistically. Restore the
    // persisted view when the backend rejects the write, so the form does not
    // claim a voice is configured when nothing was saved.
    await configService.reload();
    throw error;
  } finally {
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent(TEXT_TO_SPEECH_CONFIG_CHANGED_EVENT));
    }
  }
};
