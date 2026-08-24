/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../test/setup-dom.ts';

import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import React from 'react';
import SyntaxHighlightBoundary from './SyntaxHighlightBoundary';

afterEach(() => {
  cleanup();
});

const BrokenGrammar: React.FC = () => {
  throw new TypeError('emitter.startScope is not a function');
};

describe('SyntaxHighlightBoundary', () => {
  test('contains a grammar failure and recovers when the code block changes', () => {
    const previousConsoleError = console.error;
    console.error = () => undefined;
    try {
      const { container, rerender } = render(
        <SyntaxHighlightBoundary fallback={<code data-syntax-highlight-fallback>plain code</code>} resetKey='broken'>
          <BrokenGrammar />
        </SyntaxHighlightBoundary>
      );

      expect(container.querySelector('[data-syntax-highlight-fallback]')?.textContent).toBe('plain code');

      rerender(
        <SyntaxHighlightBoundary fallback={<code data-syntax-highlight-fallback>plain code</code>} resetKey='fixed'>
          <code data-syntax-highlight-success>highlighted code</code>
        </SyntaxHighlightBoundary>
      );

      expect(container.querySelector('[data-syntax-highlight-success]')?.textContent).toBe('highlighted code');
      expect(container.querySelector('[data-syntax-highlight-fallback]')).toBeNull();
    } finally {
      console.error = previousConsoleError;
    }
  });
});
