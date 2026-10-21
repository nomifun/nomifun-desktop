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
const otherId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000973');
const input = (content = 'Original pending input'): SessionInputQueueItem => ({ message_id: messageId,
  input: { content, files: ['reference.png'], inject_skills: ['review'] }, delivery: { kind: 'queue' },
  input_delivery: { state: 'pending', placement: 'queued', event_seq: 7 }, held: false });

function mount(initial: SessionInputQueueItem | SessionInputQueueItem[], options: { changing?: boolean; error?: unknown } = {}) {
  const change = mock(async (_action: SessionInputQueueAction) => {});
  const refresh = mock(() => {});
  const draw = (items: SessionInputQueueItem[]) => <I18nextProvider i18n={i18n}><SessionInputQueuePanel snapshot={{ version: 7, items }}
    changing={options.changing ?? false} error={options.error} activeTurn={activeTurn} onChange={change} onRefresh={refresh} /></I18nextProvider>;
  const view = render(draw(Array.isArray(initial) ? initial : [initial]));
  const expand = () => fireEvent.click(view.getByRole('button', { name: /Queued messages/ }));
  const openMenu = async (index = 0) => {
    fireEvent.click(view.getAllByRole('button', { name: conversation.inputQueue.more })[index]);
    return within(await view.findByRole('menu', { name: conversation.inputQueue.more }));
  };
  const openQueueMenu = async () => {
    fireEvent.click(view.getByRole('button', { name: conversation.inputQueue.queueActions }));
    return within(await view.findByRole('menu', { name: conversation.inputQueue.queueActions }));
  };
  return { view, change, refresh, expand, openMenu, openQueueMenu, rerender: (items: SessionInputQueueItem[]) => view.rerender(draw(items)) };
}

test('idle startup and current-turn steering never render as queued messages', () => {
  const starting = input('Starting immediately');
  starting.input_delivery = { state: 'pending', placement: 'transcript', kind: 'root', event_seq: 7 };
  const steering = { ...input('Correct the current task'), message_id: otherId,
    input_delivery: { state: 'pending' as const, placement: 'steering' as const, kind: 'steering' as const, turn_id: activeTurn, event_seq: 8 } };
  const { view } = mount([starting, steering]);
  expect(view.queryByTestId('session-input-queue')).toBeNull();
});

test('Guide moves the original occurrence out of the dock only after the Host confirms its placement', async () => {
  const original = input();
  const { view, change, rerender } = mount(original);
  fireEvent.click(view.getByRole('button', { name: conversation.inputQueue.guide }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ kind: 'promote', message_id: messageId, turn_id: activeTurn }));
  expect(view.getByText(original.input.content)).toBeTruthy();
  rerender([{ ...original, delivery: { kind: 'prefer_current', turn_id: activeTurn },
    input_delivery: { state: 'pending', placement: 'steering', kind: 'steering', turn_id: activeTurn, event_seq: 8 } }]);
  expect(view.queryByTestId('session-input-queue')).toBeNull();
});

test('an expired Guide displays why it returned to the queue', () => {
  const item = input('Original exact-target supplement');
  item.delivery = { kind: 'prefer_current', turn_id: otherId };
  item.input_delivery.reason = 'TARGET_TURN_ENDED';
  const { view } = mount(item);
  expect(view.getByText(conversation.inputDelivery.deferred)).toBeTruthy();
});

test('multiple queued messages start collapsed and full-text inspection preserves content and metadata', async () => {
  const content = ('First line\n  Keep indentation and attachments\n' + '完整保留原文与换行。'.repeat(700)).slice(0, 6000);
  const items = Array.from({ length: 12 }, (_, index) => ({ ...input(index === 0 ? content : 'Pending task ' + index),
    message_id: parseMessageId('0190f5fe-7c00-7a00-8000-' + (index + 16).toString(16).padStart(12, '0')),
    input: { content: index === 0 ? content : 'Pending task ' + index, files: ['reference.png', 'brief.txt'], inject_skills: ['review', 'pdf'] },
  }));
  const { view, expand } = mount(items);
  const header = view.getByRole('button', { name: 'Queued messages (12)' });
  expect(header.getAttribute('aria-expanded')).toBe('false');
  expect(view.queryAllByTestId('session-input-queue-item')).toHaveLength(0);
  expand();
  expect(view.getAllByTestId('session-input-queue-item')).toHaveLength(12);
  fireEvent.click(within(view.getAllByTestId('session-input-queue-item')[0]).getByRole('button', { name: /^First line/ }));
  const dialog = within(await view.findByRole('dialog'));
  expect(dialog.getByTestId('session-input-queue-full-text').textContent).toBe(content);
  expect(dialog.getByText('reference.png')).toBeTruthy();
  expect(dialog.getByText('brief.txt')).toBeTruthy();
  expect(dialog.getByText(/review, pdf/)).toBeTruthy();
});

test('editing and cancelling are local; save replaces the exact pending input and leaves its metadata intact', async () => {
  const item = input();
  const { view, change, openMenu } = mount(item);
  let menu = await openMenu();
  fireEvent.click(menu.getByRole('menuitem', { name: common.edit }));
  const editor = view.getByRole('textbox', { name: conversation.inputQueue.edit }) as HTMLTextAreaElement;
  expect(editor.value).toBe(item.input.content);
  fireEvent.change(editor, { target: { value: 'Keep this edit\nand line breaks' } });
  expect(change).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: common.cancel }));
  expect(view.getByText(item.input.content)).toBeTruthy();
  expect(change).not.toHaveBeenCalled();
  menu = await openMenu();
  fireEvent.click(menu.getByRole('menuitem', { name: common.edit }));
  fireEvent.change(view.getByRole('textbox', { name: conversation.inputQueue.edit }), { target: { value: 'Revised\nprompt' } });
  fireEvent.click(view.getByRole('button', { name: common.save }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ kind: 'replace', message_id: messageId, content: 'Revised\nprompt' }));
  expect(item.input.files).toEqual(['reference.png']);
  expect(item.input.inject_skills).toEqual(['review']);
});

test('a claimed edit remains recoverable and never sends a separate message', async () => {
  const { view, change, openMenu } = mount(input());
  change.mockRejectedValueOnce(new Error('already claimed'));
  const menu = await openMenu();
  fireEvent.click(menu.getByRole('menuitem', { name: common.edit }));
  fireEvent.change(view.getByRole('textbox', { name: conversation.inputQueue.edit }), { target: { value: 'Preserved edit' } });
  fireEvent.click(view.getByRole('button', { name: common.save }));
  await waitFor(() => expect(view.getByText(conversation.inputQueue.changeFailed)).toBeTruthy());
  expect((view.getByRole('textbox', { name: conversation.inputQueue.edit }) as HTMLTextAreaElement).value).toBe('Preserved edit');
});

test('an input claimed while its editor is open cannot discard or resubmit the unsaved edit', async () => {
  const { view, change, openMenu, rerender } = mount(input());
  const menu = await openMenu();
  fireEvent.click(menu.getByRole('menuitem', { name: common.edit }));
  fireEvent.change(view.getByRole('textbox', { name: conversation.inputQueue.edit }), { target: { value: 'Keep this unsaved correction' } });
  rerender([]);
  expect((view.getByRole('textbox', { name: conversation.inputQueue.edit }) as HTMLTextAreaElement).value).toBe('Keep this unsaved correction');
  expect(view.getByRole('status').textContent).toBe(conversation.inputQueue.editUnavailable);
  expect((view.getByRole('button', { name: common.save }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(view.getByRole('button', { name: common.save }));
  expect(change).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button', { name: common.cancel }));
  expect(view.queryByTestId('session-input-queue')).toBeNull();
});

test.each(['pause', 'resume', 'clear'] as const)('the %s action has a separate, explicit queue scope', async kind => {
  const { change, openMenu, openQueueMenu } = mount({ ...input(), held: kind === 'resume' });
  const messageMenu = await openMenu();
  expect(messageMenu.queryByRole('menuitem', { name: conversation.inputQueue.pause })).toBeNull();
  expect(messageMenu.queryByRole('menuitem', { name: conversation.inputQueue.clear })).toBeNull();
  fireEvent.mouseDown(document.body);
  fireEvent.click(document.body);
  const queueMenu = await openQueueMenu();
  fireEvent.click(queueMenu.getByRole('menuitem', { name: conversation.inputQueue[kind] }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ kind }));
});

test('reordering visible queued inputs preserves hidden input identities and positions', async () => {
  const first = input();
  const hidden = { ...input('Current task supplement'), message_id: otherId,
    input_delivery: { state: 'pending' as const, placement: 'steering' as const } };
  const last = { ...input('Last queued input'), message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000974') };
  const { change, expand, openMenu } = mount([first, hidden, last]);
  expand();
  const menu = await openMenu();
  expect(menu.getByRole('menuitem', { name: conversation.inputQueue.moveUp }).getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(menu.getByRole('menuitem', { name: conversation.inputQueue.moveDown }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ kind: 'reorder', message_ids: [last.message_id, hidden.message_id, first.message_id] }));
});

test('queue controls count only queued work while starting and steering inputs remain outside their presentation', async () => {
  const starting = { ...input('Starting immediately'), message_id: otherId,
    input_delivery: { state: 'pending' as const, placement: 'transcript' as const } };
  const steering = { ...input('Already guiding the current task'), message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000974'),
    input_delivery: { state: 'pending' as const, placement: 'steering' as const } };
  const { view, change, openQueueMenu } = mount([starting, input(), steering]);
  expect(view.getByRole('region', { name: 'Queued messages (1)' })).toBeTruthy();
  expect(view.queryByText(starting.input.content)).toBeNull();
  expect(view.queryByText(steering.input.content)).toBeNull();
  const menu = await openQueueMenu();
  fireEvent.click(menu.getByRole('menuitem', { name: conversation.inputQueue.clear }));
  await waitFor(() => expect(change).toHaveBeenCalledWith({ kind: 'clear' }));
});

test('blocked inputs retain editing and removal; changing locks mutations without hiding inspection', async () => {
  const item = input();
  item.input_delivery = { state: 'blocked', placement: 'queued', reason: 'ATTACHMENT_PREPARATION_FAILED', event_seq: 7 };
  const { view, openMenu } = mount(item, { changing: true });
  expect((view.getByRole('button', { name: conversation.inputQueue.guide }) as HTMLButtonElement).disabled).toBe(true);
  expect(view.getByText(conversation.inputQueue.blocked)).toBeTruthy();
  const menu = await openMenu();
  expect(menu.getByRole('menuitem', { name: common.edit }).getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(menu.getByRole('menuitem', { name: conversation.inputQueue.showFull }));
  expect(await view.findByRole('dialog')).toBeTruthy();
});

test('read failures keep retry visible and a failed removal retains the original row', async () => {
  const { view, change, refresh } = mount(input(), { error: new Error('read failed') });
  fireEvent.click(view.getByRole('button', { name: common.retry }));
  expect(refresh).toHaveBeenCalledTimes(1);
  change.mockRejectedValueOnce(new Error('no longer pending'));
  fireEvent.click(view.getByRole('button', { name: conversation.inputQueue.remove }));
  await waitFor(() => expect(view.getByText(conversation.inputQueue.changeFailed)).toBeTruthy());
  expect(view.getAllByRole('alert')).toHaveLength(2);
  expect(view.getAllByTestId('session-input-queue-item')).toHaveLength(1);
});
