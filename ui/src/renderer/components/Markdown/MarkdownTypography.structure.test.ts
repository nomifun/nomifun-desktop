/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const markdownSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const shadowSource = readFileSync(new URL('./ShadowView.tsx', import.meta.url), 'utf8');

/** The `<ShadowView …>` opening tag rendered by the Markdown wrapper. */
const shadowViewTag = markdownSource.match(/<ShadowView\b[^>]*>/)?.[0] ?? '';

describe('Markdown typography controls', () => {
  test('lets message surfaces override the Shadow DOM body typography', () => {
    // Both ends of the contract declare the optional overrides…
    expect(markdownSource.includes('fontSize?: string')).toBe(true);
    expect(markdownSource.includes('lineHeight?: string')).toBe(true);
    expect(shadowSource.includes('fontSize?: string')).toBe(true);
    expect(shadowSource.includes('lineHeight?: string')).toBe(true);

    // …and the wrapper actually forwards them. Asserted per prop rather than as
    // one literal tag so that adding an unrelated prop cannot silently pass.
    expect(shadowViewTag).not.toBe('');
    expect(shadowViewTag.includes('fontSize={fontSize}')).toBe(true);
    expect(shadowViewTag.includes('lineHeight={lineHeight}')).toBe(true);

    // The Shadow DOM stylesheet is what consumes them.
    expect(shadowSource.includes("const resolvedFontSize = fontSize ?? (isMobile ? '14px' : '16px');")).toBe(true);
    expect(shadowSource.includes("const resolvedLineHeight = lineHeight ?? (isMobile ? '19.6px' : '28px');")).toBe(true);
    expect(shadowSource.includes('const usesExplicitTypography = Boolean(fontSize || lineHeight);')).toBe(true);
    expect(shadowSource.includes('font-size:${resolvedFontSize};')).toBe(true);
    expect(shadowSource.includes('line-height:${resolvedLineHeight};')).toBe(true);
    expect(shadowSource.includes("margin-block-start: ${usesExplicitTypography ? '10px' : '16px'};")).toBe(true);
    expect(shadowSource.includes("font-size: ${usesExplicitTypography ? resolvedFontSize : '24px'};")).toBe(true);
    expect(shadowSource.includes("font-size: ${usesExplicitTypography ? resolvedFontSize : '16px'};")).toBe(true);
    // The overrides must survive the trip through the style factory.
    expect(
      shadowSource.includes('createInitStyle(currentTheme, cssVars, customCss, isMobile, fontSize, lineHeight, compact)')
    ).toBe(true);
  });

  test('forwards the compact preset without dropping the typography overrides', () => {
    // `compact` was added alongside the overrides; it is a third, independent
    // knob and must not replace the forwarding of fontSize/lineHeight.
    expect(markdownSource.includes('compact?: boolean;')).toBe(true);
    expect(shadowSource.includes('compact?: boolean;')).toBe(true);
    expect(shadowViewTag.includes('compact={compact}')).toBe(true);
    expect(shadowSource.includes('const compactTypographyCss = compact')).toBe(true);
    // Recomputing the stylesheet must depend on every typography input.
    const styleDeps = shadowSource.match(/\[compact,[^\]]*\]/)?.[0] ?? '';
    expect(styleDeps.includes('fontSize')).toBe(true);
    expect(styleDeps.includes('lineHeight')).toBe(true);
  });
});
