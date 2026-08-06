/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ProviderId } from '@/common/types/ids';

export type SpeechToTextProvider = 'openai' | 'deepgram';

export type OpenAISpeechToTextConfig = {
  api_key: string;
  base_url?: string;
  language?: string;
  model: string;
  prompt?: string;
  temperature?: number;
};

export type DeepgramSpeechToTextConfig = {
  api_key: string;
  base_url?: string;
  detectLanguage?: boolean;
  language?: string;
  model: string;
  punctuate?: boolean;
  smartFormat?: boolean;
};

/**
 * Install-wide speech-to-text default (`tools.speechToText`).
 *
 * `openai` / `deepgram` are RETIRED embedded-credential blocks: the backend has
 * executed transcription by `(provider_id, model)` since the catalog migration
 * and refuses those shapes, so `normalizeSpeechToTextConfig` strips them the
 * first time the section is opened rather than keeping a dead API key on disk.
 */
export type SpeechToTextConfig = {
  autoSend?: boolean;
  enabled: boolean;
  provider: SpeechToTextProvider;
  provider_id?: ProviderId;
  language?: string;
  model?: string;
  deepgram?: DeepgramSpeechToTextConfig;
  openai?: OpenAISpeechToTextConfig;
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
  provider: SpeechToTextProvider;
  text: string;
};
