/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, mock, spyOn, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach } from 'bun:test';

import ThinkingProcessDisplay from './ThinkingProcessDisplay';

afterEach(cleanup);

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

  test('a phase with no elapsed time keeps its status and body without starting a clock', () => {
    const interval = spyOn(globalThis, 'setInterval');
    const formatTime = mock(() => '9 seconds');
    try {
      const page = render(<ThinkingProcessDisplay state='running' variant='process' showElapsedTime={false}
        runningFallbackLabel='Thinking' content='Full phase reasoning' formatElapsedTime={formatTime} />);
      expect(page.getByRole('button').textContent).toBe('Thinking');
      expect(page.getByRole('button').getAttribute('aria-expanded')).toBe('true');
      expect(page.container.textContent).toContain('Full phase reasoning');
      expect(formatTime).not.toHaveBeenCalled();
      expect(interval).not.toHaveBeenCalled();
      page.unmount();
    } finally {
      interval.mockRestore();
    }
  });

  test('applies the configured body length and completed excerpt without discarding content', () => {
    const html = renderToStaticMarkup(
      <ThinkingProcessDisplay
        state='completed'
        content='完整思考内容'
        completedLabel='思考完成'
        completedSummary='检查了关键路径'
        bodyLength='compact'
      />
    );

    expect(html.includes('data-thinking-body-length="compact"')).toBe(true);
    expect(html.includes('思考完成 · 检查了关键路径')).toBe(true);
    expect(html.includes('完整思考内容')).toBe(true);
  });

  test('preserves manual thinking expansion across streaming completion and resets for a new identity', () => {
    const view = (state: 'running' | 'completed', identityKey = 'step-1') =>
      <ThinkingProcessDisplay state={state} identityKey={identityKey} variant='process' content='Full reasoning' />;
    const page = render(view('running'));
    const toggle = page.getByRole('button');
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    page.rerender(view('completed'));
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(toggle.getAttribute('aria-controls')).toBe(page.container.querySelector('[data-thinking-process-body]')?.id);
    page.rerender(view('completed', 'step-2'));
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(page.container.textContent).toContain('Full reasoning');
  });
});
