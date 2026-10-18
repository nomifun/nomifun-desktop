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
test('a saved pending input is visible beside its original user bubble without declaring execution', () => {
  const page = mount({ ...base, content: { ...base.content, input_delivery: { state: 'pending' } } });
  expect(page.getByTestId('message-input-delivery').textContent).toBe('Saved, waiting to be processed');
  expect(page.getByTestId('message-text-content').textContent).toBe(base.content.content);
  expect(page.queryByRole('alert')).toBeNull();
  expect(page.queryByTestId('message-edit-action')).not.toBeNull();
});
test('blocked input keeps its saved body and offers a concrete corrective step', () => {
  const page = mount({ ...base, content: { ...base.content, input_delivery: { state: 'blocked', reason: 'ATTACHMENT_PREPARATION_FAILED' } } });
  expect(page.getByTestId('message-input-delivery').textContent).toContain('saved but not processed');
  expect(page.getByTestId('message-input-delivery').textContent).toContain('attachments are available');
  expect(page.queryByRole('alert')).toBeNull();
});
test('claimed history clears the pending label even when a longer live body wins presentation', () => {
  const canonical = { ...base, content: { ...base.content, input_delivery: { state: 'claimed' as const } } };
  const optimistic = { ...base, content: { content: `${base.content.content} longer live fragment`, input_delivery: { state: 'pending' as const } } };
  const merged = preferTextMessageVersion(canonical, optimistic);
  expect(merged.content.input_delivery).toEqual({ state: 'claimed' });
  const page = mount(merged);
  expect(page.queryByTestId('message-input-delivery')).toBeNull();
});
test('context-cleared input has a neutral discarded state without a pending or execution failure label', () => {
  const page = mount({ ...base, content: { ...base.content, input_delivery: { state: 'discarded', reason: 'CONTEXT_CLEARED' } } });
  expect(page.getByTestId('message-input-delivery').textContent).toBe('No longer pending after the context was cleared');
  expect(page.queryByRole('alert')).toBeNull();
  expect(page.getByTestId('message-text-content').textContent).toBe(base.content.content);
});

test('an owner-cancelled accepted input keeps its body and shows cancellation without preparation failure or original-input retry actions', () => {
  const page = mount({ ...base, content: { ...base.content, input_delivery: { state: 'cancelled', reason: 'CANCELLED_BY_OWNER', event_seq: 20 } } });
  expect(page.getByTestId('message-input-delivery').textContent).toBe(en.inputDelivery.cancelled);
  expect(page.getByTestId('message-text-content').textContent).toBe(base.content.content);
  expect(page.queryByRole('alert')).toBeNull();
  expect(page.queryByTestId('message-edit-action')).toBeNull();
  expect(page.queryByRole('button')).toBeNull();
  expect(page.queryByText(en.inputDelivery.blocked)).toBeNull();
  expect(page.queryByText(en.inputDelivery.skill)).toBeNull();
});
