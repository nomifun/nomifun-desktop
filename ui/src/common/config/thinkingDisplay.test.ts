/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import {
  buildCompletedThinkingSummary,
  normalizeThinkingContentDisplayLength,
  normalizeThinkingSummaryDisplayLength,
} from './thinkingDisplay';

describe('thinking display preferences', () => {
  test('falls back to the compatibility defaults for unknown persisted values', () => {
    expect(normalizeThinkingContentDisplayLength('unexpected')).toBe('full');
    expect(normalizeThinkingContentDisplayLength(null)).toBe('full');
    expect(normalizeThinkingSummaryDisplayLength('unexpected')).toBe('hidden');
    expect(normalizeThinkingSummaryDisplayLength(undefined)).toBe('hidden');
  });

  test('maps the retired multi-level choices onto the new binary preferences', () => {
    expect(normalizeThinkingContentDisplayLength('standard')).toBe('compact');
    expect(normalizeThinkingSummaryDisplayLength('short')).toBe('shown');
    expect(normalizeThinkingSummaryDisplayLength('standard')).toBe('shown');
    expect(normalizeThinkingSummaryDisplayLength('long')).toBe('shown');
  });

  test('uses the beginning of the reasoning content for a completed excerpt', () => {
    expect(buildCompletedThinkingSummary('fallback subject', '  Reviewing\ncontext  ', 'shown')).toBe(
      'Reviewing context'
    );
  });

  test('keeps hidden summaries empty and truncates by Unicode code point', () => {
    const longSummary = '思'.repeat(60);
    expect(buildCompletedThinkingSummary('', longSummary, 'hidden')).toBe('');
    expect(buildCompletedThinkingSummary('', longSummary, 'shown')).toBe(`${'思'.repeat(48)}…`);
  });
});
