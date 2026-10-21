import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { parseMessageId } from '@/common/types/ids';
import type { SessionInputQueueItem } from '@/common/adapter/ipcBridge';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import SessionInputQueuePanel from './SessionInputQueuePanel';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { common, conversation } } } });
afterEach(cleanup);
const activeTurn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000971');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000972');

function mount(item: SessionInputQueueItem | SessionInputQueueItem[]) {
  const change = mock(async () => {});
  const edit = mock(async (_item: SessionInputQueueItem) => {});
  const view = render(<I18nextProvider i18n={i18n}><SessionInputQueuePanel snapshot={{ version: 7, items: Array.isArray(item) ? item : [item] }} changing={false}
    activeTurn={activeTurn} onChange={change} onEdit={edit} onRefresh={() => {}} /></I18nextProvider>);
  return { view, change, edit };
}

test('a blocked queue input keeps edit and removal but cannot promise immediate delivery', () => {
  const { view, change, edit } = mount({ message_id: messageId, input: { content: 'Repair the unavailable attachment', files: ['reference.png'] },
    delivery: { kind: 'queue' }, input_delivery: { state: 'blocked', reason: 'ATTACHMENT_PREPARATION_FAILED', event_seq: 7 }, held: false });
  const immediate = view.getByRole('button', { name: conversation.inputQueue.guide }) as HTMLButtonElement;
  expect(immediate.disabled).toBe(true);
  fireEvent.click(immediate);
  expect(change).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: common.edit }));
  expect(edit).toHaveBeenCalledTimes(1);
  expect((view.getByRole('button', { name: conversation.inputQueue.remove }) as HTMLButtonElement).disabled).toBe(false);
});

test('an expired immediate target is shown as queued and only an explicit action targets the new Turn', async () => {
  const previousTurn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000973');
  const { view, change } = mount({ message_id: messageId, input: { content: 'Original exact-target supplement' },
    delivery: { kind: 'prefer_current', turn_id: previousTurn }, input_delivery: { state: 'pending', event_seq: 7 }, held: false });
  expect(view.getByText(conversation.inputQueue.queued)).toBeTruthy();
  fireEvent.click(view.getByRole('button', { name: conversation.inputQueue.guide }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ kind: 'promote', message_id: messageId, turn_id: activeTurn }));
});

test('a long multi-item queue previews compactly and full-text inspection preserves the original edit payload', async () => {
  const content = ('First line\n  Keep indentation and attachments\n' + '完整保留原文与换行。'.repeat(700)).slice(0, 6000);
  const items: SessionInputQueueItem[] = Array.from({ length: 12 }, (_, index) => ({
    message_id: parseMessageId(`0190f5fe-7c00-7a00-8000-${(index + 16).toString(16).padStart(12, '0')}`),
    input: { content: index === 0 ? content : `Pending independent task ${index}`, files: ['reference.png', 'brief.txt'] },
    delivery: { kind: 'queue' }, input_delivery: { state: 'pending', event_seq: 7 }, held: false,
  }));
  const { view, edit, change } = mount(items);
  expect(view.getAllByTestId('session-input-queue-item')).toHaveLength(12);
  expect(view.queryByTestId('session-input-queue-full-text')).toBeNull();
  fireEvent.click(view.getAllByText(conversation.inputQueue.showFull)[0]);
  expect(view.getByTestId('session-input-queue-full-text').textContent).toBe(content);
  fireEvent.click(view.getAllByRole('button', { name: common.edit })[0]);
  await waitFor(() => expect(edit).toHaveBeenCalledWith(items[0]));
  expect(edit.mock.calls[0][0].input.files).toEqual(['reference.png', 'brief.txt']);
  fireEvent.click(view.getByText(conversation.inputQueue.hideFull));
  expect(view.queryByTestId('session-input-queue-full-text')).toBeNull();
  expect(view.getAllByTestId('session-input-queue-item')).toHaveLength(12);
  expect(change).not.toHaveBeenCalled();
});
