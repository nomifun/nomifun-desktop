/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';
import { resolveSyntaxLanguage } from './syntaxLanguage';

const highlighterSource = readFileSync(new URL('./SyntaxHighlighter.ts', import.meta.url), 'utf8');
const codeBlockSource = readFileSync(new URL('./CodeBlock.tsx', import.meta.url), 'utf8');

describe('Markdown syntax language resolution', () => {
  test('normalizes common Markdown fence aliases to shipped grammars', () => {
    expect(resolveSyntaxLanguage('js')).toBe('javascript');
    expect(resolveSyntaxLanguage('TS')).toBe('typescript');
    expect(resolveSyntaxLanguage('shell-session')).toBe('bash');
    expect(resolveSyntaxLanguage('html')).toBe('markup');
    expect(resolveSyntaxLanguage('yml')).toBe('yaml');
    expect(resolveSyntaxLanguage('dockerfile')).toBe('docker');
    expect(resolveSyntaxLanguage('c++')).toBe('cpp');
    expect(resolveSyntaxLanguage('patch')).toBe('diff');
  });

  test('renders diagnostic and unknown fences as plain text without auto-detection', () => {
    for (const language of ['log', 'console', 'error', 'stack', 'unknown-language', undefined]) {
      expect(resolveSyntaxLanguage(language)).toBe('text');
    }
  });

  test('keeps the conversation renderer off the incompatible Highlight.js path', () => {
    expect(highlighterSource.includes("from 'react-syntax-highlighter/dist/esm/prism-light'")).toBe(true);
    expect(highlighterSource.includes('/languages/hljs/')).toBe(false);
    expect(codeBlockSource.includes('resolveSyntaxLanguage(language)')).toBe(true);
    expect(codeBlockSource.includes('<SyntaxHighlightBoundary')).toBe(true);
  });
});
