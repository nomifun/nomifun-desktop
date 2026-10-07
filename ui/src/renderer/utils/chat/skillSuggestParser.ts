/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Strip [SKILL_SUGGEST] blocks from content for clean display.
 */
export function stripSkillSuggest(text: string): string {
  if (!text || typeof text !== 'string') return text;
  return text
    .replace(/\[SKILL_SUGGEST\][\s\S]*?\[\/SKILL_SUGGEST\]/gi, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

/**
 * Check if content contains a [SKILL_SUGGEST] block.
 */
export function hasSkillSuggest(text: string): boolean {
  return /\[SKILL_SUGGEST\]/i.test(text);
}
