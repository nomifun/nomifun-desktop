/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { IMessageText, IMessageTips } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { ConversationProvider, type ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import { emitter } from '@/renderer/utils/emitter';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import { MessageListProvider } from '../hooks';
import MessageTips from './MessageTips';

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'en-US',
  resources: { 'en-US': { translation: { conversation, common, settings: { oneClickFeedback: 'Report Issue' } } } },
  interpolation: { escapeValue: false },
});

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000051');
const sourceId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000052');
const request: IMessageText = {
  id: 'request', message_id: sourceId, conversation_id: conversationId,
  type: 'text', position: 'right', created_at: 1, content: { content: 'Please continue this task.' },
};
const error: IMessageTips = {
  id: 'error', conversation_id: conversationId, type: 'tips', position: 'left', created_at: 2,
  content: {
    type: 'error', content: 'Stream ended',
    error: { code: 'NOMIFUN_STREAM_BROKEN', message: 'Stream ended', detail: 'Diagnostic text', retryable: true },
  },
};

function mount(message = error, context: Partial<ConversationContextValue> = {}) {
  const result = render(
    <I18nextProvider i18n={testI18n}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', ...context }}>
        <MessageListProvider initialValue={[request, message]}>
          <MessageTips message={message} />
        </MessageListProvider>
      </ConversationProvider>
    </I18nextProvider>
  );
  return { ...result, page: within(result.container) };
}

afterEach(() => { cleanup(); mock.restore(); });

describe('compact message errors', () => {
  test('starts collapsed and exposes the full diagnosis and feedback only when expanded', () => {
    const { page } = mount();
    expect(page.getByRole('alert').textContent).toBe('Response interrupted');
    const toggle = page.getByRole('button', { name: 'Show error details' });
    const panel = document.getElementById(toggle.getAttribute('aria-controls')!)!;
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    expect(panel.hidden).toBe(true);
    expect(page.queryByRole('button', { name: 'Report Issue' })).toBeNull();
    fireEvent.click(toggle);
    expect(toggle.getAttribute('aria-expanded')).toBe('true');
    expect(panel.hidden).toBe(false);
    expect(panel.textContent).toContain('NOMIFUN_STREAM_BROKEN');
    expect(panel.textContent).toContain('Diagnostic text');
    expect(page.getByRole('button', { name: 'Report Issue' })).toBeDefined();
    fireEvent.click(page.getByRole('button', { name: 'Hide error details' }));
    expect(panel.hidden).toBe(true);
  });

  test('retry still recalls the original request without expanding details', () => {
    const emit = spyOn(emitter, 'emit');
    const { page } = mount();
    fireEvent.click(page.getByRole('button', { name: 'Retry' }));
    expect(emit).toHaveBeenCalledWith('sendbox.edit', {
      msgId: sourceId, createdAt: 1, content: 'Please continue this task.',
    });
    expect(page.getByRole('button', { name: 'Show error details' }).getAttribute('aria-expanded')).toBe('false');
  });

  test.each([{ isProcessing: true }, { readOnly: true }])('does not offer retry in a restricted context %j', (context) => {
    const { page } = mount(error, context);
    expect(page.queryByRole('button', { name: 'Retry' })).toBeNull();
    expect(page.getByRole('button', { name: 'Show error details' })).toBeDefined();
  });

  test('non-retryable errors keep their reason available', () => {
    const { page } = mount({ ...error, content: { ...error.content, error: { ...error.content.error!, retryable: false } } });
    expect(page.queryByRole('button', { name: 'Retry' })).toBeNull();
    fireEvent.click(page.getByRole('button', { name: 'Show error details' }));
    expect(page.getByText('Needs configuration')).toBeDefined();
  });

  test('truncation keeps its dedicated continuation action disabled while processing', () => {
    const { page } = mount({
      ...error,
      content: {
        ...error.content,
        error: { ...error.content.error!, code: 'OUTPUT_TRUNCATED' },
        recovery: { kind: 'continue_truncated', source_message_id: sourceId, failure_code: 'output_truncated' },
      },
    }, { isProcessing: true });
    expect(page.queryByRole('button', { name: 'Retry' })).toBeNull();
    expect(page.getByRole('button', { name: 'Continue execution' }).hasAttribute('disabled')).toBe(true);
  });

  test.each(['Legacy diagnostic text', '{"error":"Legacy JSON diagnostic"}'])('keeps legacy details recoverable: %s', (content) => {
    const { page } = mount({ ...error, content: { type: 'error', content } });
    expect(page.getByRole('alert').textContent).toBe(conversation.agentError.fallbackTitle);
    const toggle = page.getByRole('button', { name: 'Show error details' });
    const panel = document.getElementById(toggle.getAttribute('aria-controls')!)!;
    expect(panel.hidden).toBe(true);
    fireEvent.click(toggle);
    expect(panel.hidden).toBe(false);
    expect(panel.textContent).toContain(content.startsWith('{') ? 'Legacy JSON diagnostic' : content);
  });

  test('each error controls its own details panel', () => {
    const first = mount();
    const second = mount({ ...error, id: 'second-error' });
    const firstToggle = first.page.getByRole('button', { name: 'Show error details' });
    const secondToggle = second.page.getByRole('button', { name: 'Show error details' });
    expect(firstToggle.getAttribute('aria-controls')).not.toBe(secondToggle.getAttribute('aria-controls'));
    fireEvent.click(firstToggle);
    expect(secondToggle.getAttribute('aria-expanded')).toBe('false');
  });
});
