/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

import ThinkingProcessDisplay from './ThinkingProcessDisplay';

describe('ThinkingProcessDisplay', () => {
  test('renders a header-only live activity without inventing thinking content', () => {
    const html = renderToStaticMarkup(
      <ThinkingProcessDisplay
        state='running'
        subject='正在分析当前画布'
        identityKey='assistant-1'
        disclosure={false}
        formatElapsedTime={() => '0s'}
        role='status'
      />
    );

    expect(html.includes('data-thinking-process-state="running"')).toBe(true);
    expect(html.includes('data-thinking-process-disclosure="false"')).toBe(true);
    expect(html.includes('role="status"')).toBe(true);
    expect(html.includes('正在分析当前画布 · 0s')).toBe(true);
    expect(html.includes('data-thinking-process-body')).toBe(false);
    expect(html.includes('data-thinking-process-toggle')).toBe(false);
  });

  test('keeps completed desktop thinking available as a collapsible body', () => {
    const html = renderToStaticMarkup(
      <ThinkingProcessDisplay
        state='completed'
        subject='ignored after completion'
        content='已检查上下文'
        identityKey='thinking-1'
        completedLabel='思考完成'
      />
    );

    expect(html.includes('data-thinking-process-state="completed"')).toBe(true);
    expect(html.includes('data-thinking-process-disclosure="true"')).toBe(true);
    expect(html.includes('思考完成')).toBe(true);
    expect(html.includes('data-thinking-process-body')).toBe(true);
    expect(html.includes('已检查上下文')).toBe(true);
  });
});
