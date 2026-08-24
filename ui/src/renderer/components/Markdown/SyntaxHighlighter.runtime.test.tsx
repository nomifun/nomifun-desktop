/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import SyntaxHighlighter from './SyntaxHighlighter';
import { resolveSyntaxLanguage } from './syntaxLanguage';

describe('Markdown syntax highlighter runtime', () => {
  test('highlights a common alias through the Prism grammar registry', () => {
    const html = renderToStaticMarkup(
      <SyntaxHighlighter language={resolveSyntaxLanguage('js')} PreTag='div'>
        {'const answer = 42;\n'}
      </SyntaxHighlighter>
    );

    expect(html.includes('const')).toBe(true);
    expect(html.includes('answer')).toBe(true);
    expect(html.includes('token keyword')).toBe(true);
  });

  test('renders copied error output as plain text', () => {
    const html = renderToStaticMarkup(
      <SyntaxHighlighter language={resolveSyntaxLanguage('log')} PreTag='div'>
        {'TypeError: emitter.startScope is not a function\n    at highlightAuto (...)'}
      </SyntaxHighlighter>
    );

    expect(html.includes('TypeError: emitter.startScope is not a function')).toBe(true);
    expect(html.includes('highlightAuto')).toBe(true);
  });
});
