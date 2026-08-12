/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Candidate voice ids offered by the TTS variant of `TaskModelSelect`.
 *
 * The field is free text on purpose — every provider names its voices
 * differently and new ones ship constantly, so a closed list would go stale and
 * block a voice that works. Suggestions are keyed by the exact persisted
 * speech-synthesis protocol, never by provider platform: one provider may host
 * models backed by different adapters. Anything else gets an empty candidate
 * list and the user types the id.
 */
const STEPFUN_SYSTEM_VOICES = [
    'cixingnansheng',
    'boyinnansheng',
    'wenrounansheng',
    'shenchennanyin',
    'yuanqinansheng',
    'zhengpaiqingnian',
    'qingniandaxuesheng',
    'wenrougongzi',
    'ruyananshi',
    'jingdiannvsheng',
    'wenrounvsheng',
    'qingchunshaonv',
    'yuanqishaonv',
    'jilingshaonv',
    'tianmeinvsheng',
    'ruanmengnvsheng',
    'linjiajiejie',
    'linjiameimei',
    'zhixingjiejie',
    'shuangkuaijiejie',
    'wenjingxuejie',
    'lengyanyujie',
    'qinqienvsheng',
    'youyanvsheng',
  ] as const;

export const TTS_VOICE_OPTIONS_BY_PROTOCOL: Record<string, readonly string[]> = {
  'openai.audio_speech': ['alloy', 'echo', 'fable', 'onyx', 'nova', 'shimmer'],
  // StepFun (阶跃星辰) system voices for the current
  // `stepaudio-2.5-tts` surface. Both the metered API and Step Plan use these
  // ids. The control remains free text so newly published or cloned voices do
  // not have to wait for an application release.
  'stepfun.audio_speech': STEPFUN_SYSTEM_VOICES,
};

const SILICONFLOW_SYSTEM_VOICES = [
  'alex',
  'benjamin',
  'charles',
  'david',
  'anna',
  'bella',
  'claire',
  'diana',
] as const;

/**
 * SiliconFlow system voices are model-scoped ids. Keep this map exact and
 * deliberately small: synthesizing `<any-model>:alex` would make a plausible
 * looking value that the selected model may not support. Unknown/new models
 * get no guesses and retain the free-text field for a current system id or a
 * user-created `speech:...` URI.
 */
const SILICONFLOW_SYSTEM_VOICES_BY_MODEL: Readonly<Record<string, readonly string[]>> = {
  'FunAudioLLM/CosyVoice2-0.5B': SILICONFLOW_SYSTEM_VOICES.map(
    (voice) => `FunAudioLLM/CosyVoice2-0.5B:${voice}`
  ),
  'fnlp/MOSS-TTSD-v0.5': SILICONFLOW_SYSTEM_VOICES.map(
    (voice) => `fnlp/MOSS-TTSD-v0.5:${voice}`
  ),
};

export const ttsVoiceOptionsFor = (
  protocol: string | undefined,
  model?: string
): readonly string[] => {
  if (protocol === 'siliconflow.audio_speech') {
    return SILICONFLOW_SYSTEM_VOICES_BY_MODEL[model?.trim() ?? ''] ?? [];
  }
  return (protocol && TTS_VOICE_OPTIONS_BY_PROTOCOL[protocol]) || [];
};

/** Providers whose selected TTS model id already identifies the voice. */
export const ttsUsesModelIdAsVoice = (protocol: string | undefined): boolean =>
  protocol === 'deepgram.speak_rest';
