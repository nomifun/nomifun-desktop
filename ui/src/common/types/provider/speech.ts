/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ProviderId } from '@/common/types/ids';

/**
 * Install-wide speech-to-text default (`tools.speechToText`).
 * Provider credentials and transport live exclusively on the selected model's
 * speech_recognition capability.
 */
export type SpeechToTextConfig = {
  autoSend?: boolean;
  enabled: boolean;
  provider_id?: ProviderId;
  language?: string;
  model?: string;
};

/**
 * Install-wide speech-synthesis default (`tools.textToSpeech`).
 *
 * Deliberately parallel to {@link SpeechToTextConfig} minus the `enabled`
 * switch: synthesis has no input-box affordance to gate, so the key's presence
 * IS the configuration. `voice` is free text — provider voice ids differ and
 * change often.
 */
export type TextToSpeechConfig = {
  provider_id: ProviderId;
  model: string;
  voice: string | null;
};

export type SpeechToTextAudioBuffer = Uint8Array | number[] | Record<string, number>;

export type SpeechToTextRequest = {
  audioBuffer: SpeechToTextAudioBuffer;
  file_name: string;
  languageHint?: string;
  mimeType: string;
};

export type SpeechToTextResult = {
  language?: string;
  model: string;
  /** Resolved runtime platform, not a hard-coded two-provider enum. */
  provider: string;
  text: string;
};
