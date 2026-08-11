/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const markdownSource = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
const shadowSource = readFileSync(new URL('./ShadowView.tsx', import.meta.url), 'utf8');
const typographyCss = readFileSync(new URL('./MarkdownTypography.css', import.meta.url), 'utf8');
const previewSource = readFileSync(
  new URL('../../pages/conversation/Preview/components/viewers/MarkdownViewer.tsx', import.meta.url),
  'utf8'
);

const shadowViewTag = markdownSource.match(/<ShadowView\b[^>]*>/)?.[0] ?? '';

describe('shared Markdown typography contract', () => {
  test('keeps message typography overrides flowing into the Shadow DOM', () => {
    expect(markdownSource.includes('fontSize?: string')).toBe(true);
    expect(markdownSource.includes('lineHeight?: string')).toBe(true);
    expect(shadowSource.includes('fontSize?: string')).toBe(true);
    expect(shadowSource.includes('lineHeight?: string')).toBe(true);

    expect(shadowViewTag).not.toBe('');
    expect(shadowViewTag.includes('fontSize={fontSize}')).toBe(true);
    expect(shadowViewTag.includes('lineHeight={lineHeight}')).toBe(true);

    expect(shadowSource.includes("const resolvedFontSize = fontSize ?? (isMobile ? '14px' : '16px');")).toBe(true);
    expect(shadowSource.includes("const resolvedLineHeight = lineHeight ?? (isMobile ? '19.6px' : '28px');")).toBe(
      true
    );
    expect(shadowSource.includes('--markdown-body-font-size: ${resolvedFontSize};')).toBe(true);
    expect(shadowSource.includes('--markdown-body-line-height: ${resolvedLineHeight};')).toBe(true);
    expect(markdownSource.includes("'markdown-article--explicit': Boolean(fontSize || lineHeight)")).toBe(true);
  });

  test('defines the knowledge document density once and injects it into ShadowView', () => {
    expect(shadowSource.includes("import markdownTypographyCss from './MarkdownTypography.css?raw';")).toBe(true);
    expect(shadowSource.includes('${markdownTypographyCss}')).toBe(true);
    expect(markdownSource.includes("'markdown-article--compact': compact")).toBe(true);

    expect(typographyCss.includes('.markdown-article--compact')).toBe(true);
    expect(typographyCss.includes('--markdown-body-font-size: 14px;')).toBe(true);
    expect(typographyCss.includes('--markdown-body-line-height: 22px;')).toBe(true);
    expect(typographyCss.includes('--markdown-h1-font-size: 22px;')).toBe(true);
    expect(typographyCss.includes('--markdown-h2-font-size: 18px;')).toBe(true);
    expect(typographyCss.includes('--markdown-paragraph-margin: 8px;')).toBe(true);
    expect(typographyCss.includes('--markdown-list-item-margin: 2px;')).toBe(true);
  });

  test('uses the same compact article contract in the workspace PreviewPanel', () => {
    expect(previewSource.includes("import '@/renderer/components/Markdown/MarkdownTypography.css';")).toBe(true);
    expect(previewSource.includes("import CodeBlock from '@/renderer/components/Markdown/CodeBlock';")).toBe(true);
    expect(previewSource.includes("className='markdown-article markdown-article--compact'")).toBe(true);
    expect(previewSource.includes('showMermaidOpenInPanelButton={false}')).toBe(true);
    expect(previewSource.includes("'overflow-auto p-20px md:p-24px text-t-primary'")).toBe(true);
    expect(previewSource.includes('overflow-auto p-32px')).toBe(false);
  });
});
