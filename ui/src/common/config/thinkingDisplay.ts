/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** Visible height of a reasoning body inside the conversation timeline. */
export type ThinkingContentDisplayLength = 'compact' | 'full';

/** Length of the optional excerpt shown beside the completed label. */
export type ThinkingSummaryDisplayLength = 'hidden' | 'shown';

export interface ThinkingDisplayPreferences {
  visible: boolean;
  contentLength: ThinkingContentDisplayLength;
  summaryLength: ThinkingSummaryDisplayLength;
}

export const DEFAULT_THINKING_DISPLAY_PREFERENCES: ThinkingDisplayPreferences = {
  // Preserve the pre-preference behavior for existing installations.
  visible: true,
  contentLength: 'full',
  summaryLength: 'hidden',
};

const CONTENT_LENGTHS = new Set<ThinkingContentDisplayLength>(['compact', 'full']);
const SUMMARY_LENGTHS = new Set<ThinkingSummaryDisplayLength>(['hidden', 'shown']);

export const normalizeThinkingContentDisplayLength = (value: unknown): ThinkingContentDisplayLength =>
  value === 'standard'
    ? 'compact'
    : typeof value === 'string' && CONTENT_LENGTHS.has(value as ThinkingContentDisplayLength)
    ? (value as ThinkingContentDisplayLength)
    : DEFAULT_THINKING_DISPLAY_PREFERENCES.contentLength;

export const normalizeThinkingSummaryDisplayLength = (value: unknown): ThinkingSummaryDisplayLength =>
  value === 'short' || value === 'standard' || value === 'long'
    ? 'shown'
    : typeof value === 'string' && SUMMARY_LENGTHS.has(value as ThinkingSummaryDisplayLength)
    ? (value as ThinkingSummaryDisplayLength)
    : DEFAULT_THINKING_DISPLAY_PREFERENCES.summaryLength;

const SHOWN_SUMMARY_CHARACTER_LIMIT = 48;

const compactText = (value: string): string => value.replace(/\s+/g, ' ').trim();

const truncateByCodePoint = (value: string, maximum: number): string => {
  const characters = Array.from(value);
  if (characters.length <= maximum) return value;
  return `${characters.slice(0, maximum).join('').trimEnd()}…`;
};

/**
 * Builds a display-only excerpt from content already returned by the runtime.
 * It never changes the persisted reasoning text and does not invoke a model.
 */
export const buildCompletedThinkingSummary = (
  subject: string,
  content: string,
  length: ThinkingSummaryDisplayLength
): string => {
  if (length === 'hidden') return '';
  const source = compactText(content) || compactText(subject);
  if (!source) return '';
  return truncateByCodePoint(source, SHOWN_SUMMARY_CHARACTER_LIMIT);
};
