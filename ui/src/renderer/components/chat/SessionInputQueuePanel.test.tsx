import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { parseMessageId } from '@/common/types/ids';
import type { SessionInputQueueAction, SessionInputQueueItem } from '@/common/adapter/ipcBridge';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import SessionInputQueuePanel from './SessionInputQueuePanel';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { common, conversation } } } });
afterEach(cleanup);
const activeTurn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000971');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000972');
const input = (content = 'Original pending input'): SessionInputQueueItem => ({ message_id: messageId,
  input: { content, files: ['reference.png'], inject_skills: ['review'] }, delivery: { kind: 'queue' },
  input_delivery: { state: 'pending', event_seq: 7 }, held: false });

function mount(item: SessionInputQueueItem | SessionInputQueueItem[], options: { changing?: boolean; editDisabled?: boolean; error?: unknown } = {}) {
  const change = mock(async (_action: SessionInputQueueAction) => {});
  const edit = mock(async (_item: SessionInputQueueItem) => {});
  const refresh = mock(() => {});
  const view = render(<I18nextProvider i18n={i18n}><SessionInputQueuePanel snapshot={{ version: 7, items: Array.isArray(item) ? item : [item] }}
    changing={options.changing ?? false} editDisabled={options.editDisabled} error={options.error}
    activeTurn={activeTurn} onChange={change} onEdit={edit} onRefresh={refresh} /></I18nextProvider>);
  const openMenu = async (index = 0) => {
    fireEvent.click(view.getAllByRole('button', { name: conversation.inputQueue.more })[index]);
    return within(await view.findByRole('menu'));
  };
  return { view, change, edit, refresh, openMenu };
}

test('a blocked input keeps compact status, editing and removal without promising Guide delivery', async () => {
  const item = input('Repair the unavailable attachment');
  item.input_delivery = { state: 'blocked', reason: 'ATTACHMENT_PREPARATION_FAILED', event_seq: 7 };
  const { view, change, edit, openMenu } = mount(item);
  const guide = view.getByRole('button', { name: conversation.inputQueue.guide }) as HTMLButtonElement;
  expect(guide.disabled).toBe(true);
  fireEvent.click(guide);
  expect(change).not.toHaveBeenCalled();
  expect(view.getByText(conversation.inputQueue.blocked)).toBeTruthy();
  const menu = await openMenu();
  fireEvent.click(menu.getByRole('menuitem', { name: common.edit }));
  await waitFor(() => expect(edit).toHaveBeenCalledWith(item));
  expect((view.getByRole('button', { name: conversation.inputQueue.remove }) as HTMLButtonElement).disabled).toBe(false);
});

test('an expired Guide target has no redundant status line and only an explicit action targets the new Turn', async () => {
  const item = input('Original exact-target supplement');
  item.delivery = { kind: 'prefer_current', turn_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000973') };
  const { view, change } = mount(item);
  expect(view.queryByText(conversation.inputQueue.queued)).toBeNull();
  expect(view.queryByText(conversation.inputQueue.steering)).toBeNull();
  fireEvent.click(view.getByRole('button', { name: conversation.inputQueue.guide }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ kind: 'promote', message_id: messageId, turn_id: activeTurn }));
});

test('a 6000-character multi-item queue stays a single-line list and inspection preserves text, attachments and skills', async () => {
  const content = ('First line\n  Keep indentation and attachments\n' + '完整保留原文与换行。'.repeat(700)).slice(0, 6000);
  const items = Array.from({ length: 12 }, (_, index) => ({ ...input(index === 0 ? content : 'Pending independent task ' + index),
    message_id: parseMessageId('0190f5fe-7c00-7a00-8000-' + (index + 16).toString(16).padStart(12, '0')),
    input: { content: index === 0 ? content : 'Pending independent task ' + index,
      files: ['reference.png', 'brief.txt'], inject_skills: ['review', 'pdf'] },
  }));
  const { view, edit, change, openMenu } = mount(items);
  expect(view.getAllByTestId('session-input-queue-item')).toHaveLength(12);
  expect(view.queryByRole('dialog')).toBeNull();
  expect(view.queryByTestId('session-input-queue-full-text')).toBeNull();
  expect(view.queryByText(conversation.inputQueue.showFull)).toBeNull();
  fireEvent.click(within(view.getAllByTestId('session-input-queue-item')[0]).getByRole('button', { name: /^First line/ }));
  const dialog = within(await view.findByRole('dialog'));
  expect(dialog.getByTestId('session-input-queue-full-text').textContent).toBe(content);
  expect(dialog.getByText('reference.png')).toBeTruthy();
  expect(dialog.getByText('brief.txt')).toBeTruthy();
  expect(dialog.getByText(/review, pdf/)).toBeTruthy();
  fireEvent.click(dialog.getByRole('button', { name: 'Close' }));
  await waitFor(() => expect(view.queryByRole('dialog')).toBeNull());
  const menu = await openMenu();
  fireEvent.click(menu.getByRole('menuitem', { name: common.edit }));
  await waitFor(() => expect(edit).toHaveBeenCalledWith(items[0]));
  expect(edit.mock.calls[0][0].input).toEqual(items[0].input);
  expect(view.getAllByTestId('session-input-queue-item')).toHaveLength(12);
  expect(change).not.toHaveBeenCalled();
});

test.each(['pause', 'resume', 'clear'] as const)('the row menu preserves the %s queue action without a toolbar', async kind => {
  const item = { ...input(), held: kind === 'resume' };
  const { view, change, openMenu } = mount(item);
  expect(view.queryByRole('button', { name: conversation.inputQueue.pause })).toBeNull();
  expect(view.queryByRole('button', { name: conversation.inputQueue.clear })).toBeNull();
  const menu = await openMenu();
  fireEvent.click(menu.getByRole('menuitem', { name: conversation.inputQueue[kind] }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ kind }));
});

test.each([0, 1])('reordering row %s uses all original identities and disables the unavailable direction', async index => {
  const second = { ...input('Second pending input'), message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000974') };
  const items = [input(), second];
  const { change, openMenu } = mount(items);
  const menu = await openMenu(index);
  const disabled = menu.getByRole('menuitem', { name: index === 0 ? conversation.inputQueue.moveUp : conversation.inputQueue.moveDown });
  expect(disabled.getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(disabled);
  expect(change).not.toHaveBeenCalled();
  fireEvent.click(menu.getByRole('menuitem', { name: index === 0 ? conversation.inputQueue.moveDown : conversation.inputQueue.moveUp }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ kind: 'reorder', message_ids: [second.message_id, messageId] }));
});

test('changing disables mutations while full-text inspection stays available', async () => {
  const { view, edit, change, openMenu } = mount(input(), { changing: true, editDisabled: true });
  expect((view.getByRole('button', { name: conversation.inputQueue.guide }) as HTMLButtonElement).disabled).toBe(true);
  expect((view.getByRole('button', { name: conversation.inputQueue.remove }) as HTMLButtonElement).disabled).toBe(true);
  const menu = await openMenu();
  for (const name of [common.edit, conversation.inputQueue.pause, conversation.inputQueue.clear]) {
    const entry = menu.getByRole('menuitem', { name });
    expect(entry.getAttribute('aria-disabled')).toBe('true');
    fireEvent.click(entry);
  }
  expect(edit).not.toHaveBeenCalled(); expect(change).not.toHaveBeenCalled();
  fireEvent.click(menu.getByRole('menuitem', { name: conversation.inputQueue.showFull }));
  expect(await view.findByRole('dialog')).toBeTruthy();
});

test.each(['files', 'inject_skills'] as const)('empty text with %s has a recognizable preview and retains metadata', async field => {
  const item = input('');
  item.input = { content: '', [field]: field === 'files' ? ['first.png', 'brief.txt'] : ['review', 'pdf'] };
  const { view } = mount(item);
  const fallback = i18n.t(field === 'files' ? 'conversation.inputQueue.files' : 'conversation.inputQueue.skillCount', { count: 2 });
  fireEvent.click(view.getByRole('button', { name: fallback }));
  const dialog = within(await view.findByRole('dialog'));
  expect(dialog.getByTestId('session-input-queue-full-text').textContent).toBe('');
  expect(dialog.getByText(field === 'files' ? 'first.png' : /review, pdf/)).toBeTruthy();
});

test('read failures retain an accessible retry and command failures do not remove the row', async () => {
  const { view, change, refresh } = mount(input(), { error: new Error('read failed') });
  fireEvent.click(view.getByRole('button', { name: common.retry }));
  expect(refresh).toHaveBeenCalledTimes(1);
  change.mockRejectedValueOnce(new Error('no longer pending'));
  fireEvent.click(view.getByRole('button', { name: conversation.inputQueue.remove }));
  await waitFor(() => expect(view.getByText(conversation.inputQueue.changeFailed)).toBeTruthy());
  expect(view.getAllByRole('alert')).toHaveLength(2);
  expect(view.getAllByTestId('session-input-queue-item')).toHaveLength(1);
});
