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
 * block a voice that works. This table therefore holds ONLY platforms whose
 * voice ids are documented and verified; anything else gets an empty candidate
 * list and the user types the id. Offering guessed ids would be worse than
 * offering none: they look authoritative and fail at synthesis time.
 */
export const TTS_VOICE_OPTIONS_BY_PLATFORM: Record<string, readonly string[]> = {
  openai: ['alloy', 'echo', 'fable', 'onyx', 'nova', 'shimmer'],
};

export const ttsVoiceOptionsFor = (platform: string | undefined): readonly string[] =>
  (platform && TTS_VOICE_OPTIONS_BY_PLATFORM[platform]) || [];
