import '../../../../../../test/setup-dom.ts';
import { cleanup, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageText } from '@/common/chat/chatLib';
import { preferTextMessageVersion } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import en from '@/renderer/services/i18n/locales/en-US/conversation.json';
import MessageText from './MessageText';
import { MessageListProvider, normalizeDbMessage } from '../hooks';
const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: { conversation: en } } } });
const base: IMessageText = { id: 'saved-input', type: 'text', position: 'right',
  message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000052'), created_at: 1,
  conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000051'), content: { content: 'Continue with this file' } };
afterEach(cleanup);
const mount = (message: IMessageText) => {
  const persisted = normalizeDbMessage(message);
  if (persisted.type !== 'text') throw new Error('expected the original user text');
  return render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <ConversationProvider value={{ conversation_id: base.conversation_id, type: 'nomi', isProcessing: false }}>
      <MessageListProvider initialValue={[persisted]}><MessageText message={persisted} /></MessageListProvider>
    </ConversationProvider>
  </I18nextProvider></MemoryRouter>);
};
test.each([
  { state: 'pending' },
  { state: 'pending', placement: 'transcript', kind: 'root' },
  { state: 'pending', placement: 'steering', kind: 'steering' },
  { state: 'claimed', placement: 'transcript', kind: 'steering' },
  { state: 'pending', reason: 'WAITING_FOR_EFFECT_SETTLEMENT' },
  { state: 'pending', reason: 'TARGET_TURN_ENDED' },
  { state: 'blocked', reason: 'ATTACHMENT_PREPARATION_FAILED' },
  { state: 'discarded', reason: 'CONTEXT_CLEARED' },
] as const)('message metadata omits execution status for %j while keeping content and actions', input_delivery => {
  const page = mount({ ...base, content: { ...base.content, input_delivery } });
  expect(page.queryByTestId('message-input-delivery')).toBeNull();
  expect(page.queryByRole('status')).toBeNull();
  expect(page.getByTestId('message-text-content').textContent).toBe(base.content.content);
  expect(page.getByTestId('message-actions').textContent).toMatch(/\d{2}:\d{2}/);
  expect(page.getByTestId('message-copy-action')).toBeTruthy();
  expect(page.queryByTestId('message-edit-action')).not.toBeNull();
});

test('claimed history retains canonical delivery when a longer live body wins presentation', () => {
  const canonical = { ...base, content: { ...base.content, input_delivery: { state: 'claimed' as const } } };
  const optimistic = { ...base, content: { content: base.content.content + ' longer live fragment', input_delivery: { state: 'pending' as const } } };
  const merged = preferTextMessageVersion(canonical, optimistic);
  expect(merged.content.input_delivery).toEqual({ state: 'claimed' });
  const page = mount(merged);
  expect(page.queryByTestId('message-input-delivery')).toBeNull();
});

test('owner-cancelled input keeps its body without a status label or edit and retry actions', () => {
  const page = mount({ ...base, content: { ...base.content, input_delivery: { state: 'cancelled', reason: 'CANCELLED_BY_OWNER', event_seq: 20 } } });
  expect(page.queryByTestId('message-input-delivery')).toBeNull();
  expect(page.getByTestId('message-text-content').textContent).toBe(base.content.content);
  expect(page.queryByRole('alert')).toBeNull();
  expect(page.queryByTestId('message-edit-action')).toBeNull();
  expect(page.queryByRole('button')).toBeNull();
});
