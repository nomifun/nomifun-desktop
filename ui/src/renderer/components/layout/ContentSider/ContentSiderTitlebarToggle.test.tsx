/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../test/setup-dom.ts';
import { afterEach, expect, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { agentSiderChannel } from '@/renderer/utils/workspace/agentSiderEvents';
import { sessionSiderChannel } from '@/renderer/utils/workspace/sessionSiderEvents';
import ContentSiderTitlebarToggle from './ContentSiderTitlebarToggle';
import { createContentSiderChannel } from './createContentSiderChannel';

afterEach(cleanup);

test('reads a state published before mounting and preserves session and agent toggle events', () => {
  sessionSiderChannel.dispatchState(true);
  agentSiderChannel.dispatchState(false);
  const received: string[] = [];
  const onToggle = (event: Event) => received.push(event.type);
  window.addEventListener('nomifun-session-sider-toggle', onToggle);
  window.addEventListener('nomifun-agent-sider-toggle', onToggle);
  try {
    const view = render(<ContentSiderTitlebarToggle channel={sessionSiderChannel} expandLabel='展开会话' collapseLabel='收起会话' />);
    const button = view.getByRole('button', { name: '展开会话' });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(button.classList.contains('app-titlebar__button--nav')).toBe(true);
    fireEvent.click(button);
    expect(received).toEqual(['nomifun-session-sider-toggle']);
    act(() => sessionSiderChannel.dispatchState(false));
    expect(view.getByRole('button', { name: '收起会话' })).toBe(button);

    view.rerender(<ContentSiderTitlebarToggle channel={agentSiderChannel} expandLabel='展开 Agent' collapseLabel='收起 Agent' />);
    fireEvent.click(view.getByRole('button', { name: '收起 Agent' }));
    expect(received).toEqual(['nomifun-session-sider-toggle', 'nomifun-agent-sider-toggle']);
    act(() => sessionSiderChannel.dispatchState(true));
    expect(view.getByRole('button', { name: '收起 Agent' }).getAttribute('aria-expanded')).toBe('true');
  } finally {
    cleanup();
    window.removeEventListener('nomifun-session-sider-toggle', onToggle);
    window.removeEventListener('nomifun-agent-sider-toggle', onToggle);
    sessionSiderChannel.dispatchState(false);
    agentSiderChannel.dispatchState(false);
  }
});

test('hides unavailable controls and immediately restores their current state', () => {
  const channel = createContentSiderChannel('test-workbench', false);
  const view = render(<ContentSiderTitlebarToggle channel={channel} expandLabel='展开' collapseLabel='收起' />);
  expect(view.queryByRole('button')).toBeNull();
  act(() => channel.dispatchState(true));
  expect(view.getByRole('button', { name: '展开' })).toBeTruthy();
  act(() => channel.setUnavailable());
  expect(view.queryByRole('button')).toBeNull();
  act(() => channel.dispatchState(false));
  expect(view.getByRole('button', { name: '收起' })).toBeTruthy();
});
