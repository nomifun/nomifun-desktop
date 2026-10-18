/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { ConfigProvider, Message } from '@arco-design/web-react';
import enUS from '@arco-design/web-react/es/locale/en-US';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { ComponentProps } from 'react';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { ICompanionThread } from '@/common/adapter/ipcBridge';
import { parseCompanionId, parseConversationId } from '@/common/types/ids';
import nomi from '@/renderer/services/i18n/locales/en-US/nomi.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import CompanionSessionToolbar from './CompanionSessionToolbar';

const i18n = createInstance();
await i18n.init({ lng: 'en-US', resources: { 'en-US': { translation: { nomi, conversation, common } } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });

const targetId = parseConversationId('019b0000-0000-7000-8000-000000000001');
const receiverId = parseConversationId('019b0000-0000-7000-8000-000000000002');
const archivedId = parseConversationId('019b0000-0000-7000-8000-000000000003');
const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000004');
type Props = ComponentProps<typeof CompanionSessionToolbar>;
const initialItems: ICompanionThread[] = [
  { conversation_id: targetId, companion_id: companionId, title: 'Target chat', created_at: 1000, updated_at: 2000, role: 'companion', archived: false },
  { conversation_id: archivedId, companion_id: companionId, title: 'Earlier archived chat', created_at: 1000, updated_at: 1500, role: 'robot', archived: true },
];

function fixture() {
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  restores.push(() => success.mockRestore(), () => error.mockRestore());
  const onRename = mock(async (_id: Props['selectedId'], _name: string) => {});
  const onArchive = mock(async (_id: Props['selectedId'], _archived: boolean) => {});
  const onDelete = mock(async (_id: Props['selectedId']) => {});
  const onContinue = mock(() => {});
  const onCreate = mock(() => {});
  const onSelect = mock(() => {});
  const props: Props = {
    items: initialItems, selectedId: targetId, selectedName: 'Target chat', selectedLoaded: true, entryKind: 'companion', activeDesktopId: receiverId,
    creating: false, routing: false, managing: false, canCreate: true, failed: false,
    onRename, onArchive, onDelete, onContinue, onCreate, onSelect,
    onClear: async () => {}, onRefresh: () => {},
  };
  const ui = (patch: Partial<Props> = {}) => <ConfigProvider locale={enUS}><I18nextProvider i18n={i18n}>
    <CompanionSessionToolbar {...props} {...patch} />
  </I18nextProvider></ConfigProvider>;
  const view = render(ui());
  const menu = async (name: string) => {
    fireEvent.click(view.getByRole('button', { name: 'More' }));
    const item = await view.findByRole('menuitem', { name });
    await act(async () => { fireEvent.click(item); });
  };
  return { view, ui, menu, onRename, onArchive, onDelete, onContinue, onCreate, onSelect, success, error };
}

test('rename cancellation sends nothing and confirmation renames the exact selected Session without changing the input target', async () => {
  const f = fixture();
  fireEvent.click(f.view.getByRole('button', { name: 'Rename' }));
  let dialog = await f.view.findByRole('dialog');
  fireEvent.change(within(dialog).getByRole('textbox', { name: 'Please enter a new name' }), { target: { value: 'Cancelled edit' } });
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(f.view.queryByRole('dialog') === null).toBe(true));
  expect(f.onRename).not.toHaveBeenCalled();

  fireEvent.click(f.view.getByRole('button', { name: 'Rename' }));
  dialog = await f.view.findByRole('dialog');
  fireEvent.change(within(dialog).getByRole('textbox', { name: 'Please enter a new name' }), { target: { value: '  Reviewed conversation name  ' } });
  fireEvent.click(within(dialog).getByRole('button', { name: 'OK' }));
  await waitFor(() => expect(f.view.queryByRole('dialog') === null).toBe(true));
  expect(f.onRename).toHaveBeenCalledTimes(1);
  expect(f.onRename.mock.calls[0]).toEqual([targetId, 'Reviewed conversation name']);
  expect(f.onContinue).not.toHaveBeenCalled();
  expect(f.onCreate).not.toHaveBeenCalled();
  expect(f.onSelect).not.toHaveBeenCalled();
  expect(f.error).not.toHaveBeenCalled();
});

test('the verified current Session can be renamed outside the list cache and its canonical name wins over a stale list title', async () => {
  const f = fixture();
  for (const items of [[], initialItems]) {
    f.view.rerender(f.ui({ items, selectedName: 'Current saved name' }));
    expect(f.view.container.textContent).toContain('Current saved name');
    expect(f.view.container.textContent).not.toContain('Target chat');
    fireEvent.click(f.view.getByRole('button', { name: 'Rename' }));
    const dialog = await f.view.findByRole('dialog');
    expect((within(dialog).getByRole('textbox') as HTMLInputElement).value).toBe('Current saved name');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(f.view.queryByRole('dialog') === null).toBe(true));
  }
  f.view.rerender(f.ui({ items: [], selectedLoaded: false, selectedName: undefined }));
  expect((f.view.getByRole('button', { name: 'Rename' }) as HTMLButtonElement).disabled).toBe(true);
  expect(f.onRename).not.toHaveBeenCalled();
});

test('rename validates Unicode byte limits, ignores IME confirmation, and accepts Enter after a valid edit', async () => {
  const f = fixture();
  fireEvent.click(f.view.getByRole('button', { name: 'Rename' }));
  const dialog = await f.view.findByRole('dialog');
  const input = within(dialog).getByRole('textbox');
  for (const name of ['界'.repeat(67), '😀'.repeat(51)]) {
    fireEvent.change(input, { target: { value: name } });
    expect(within(dialog).getByRole('alert').textContent).toBe(conversation.history.renameTooLong);
    expect((within(dialog).getByRole('button', { name: 'OK' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 });
    expect(f.onRename).not.toHaveBeenCalled();
  }
  const validName = `${'界'.repeat(66)}ab`;
  fireEvent.change(input, { target: { value: `  ${validName}  ` } });
  expect(within(dialog).queryByRole('alert') === null).toBe(true);
  fireEvent.keyDown(input, { key: 'Enter', keyCode: 13, isComposing: true });
  expect(f.onRename).not.toHaveBeenCalled();
  fireEvent.keyDown(input, { key: 'Enter', keyCode: 13 });
  await waitFor(() => expect(f.view.queryByRole('dialog') === null).toBe(true));
  expect(f.onRename.mock.calls).toEqual([[targetId, validName]]);
  expect(f.onContinue).not.toHaveBeenCalled();
  expect(f.onSelect).not.toHaveBeenCalled();
});

test('archive keeps the selected archived Session visible, showing archives reveals others, and restore targets the same Session', async () => {
  const f = fixture();
  await f.menu('Archive conversation');
  await waitFor(() => expect(f.onArchive).toHaveBeenCalledTimes(1));
  expect(f.onArchive.mock.calls[0]).toEqual([targetId, true]);
  const archivedItems = initialItems.map(item => item.conversation_id === targetId ? { ...item, archived: true } : item);
  f.view.rerender(f.ui({ items: archivedItems, selectedArchived: true }));
  const openSelect = () => fireEvent.click(f.view.container.querySelector('.arco-select-view')!);
  openSelect();
  expect(await f.view.findByRole('option', { name: 'Target chat · Archived' })).toBeTruthy();
  expect(f.view.queryByRole('option', { name: 'Earlier archived chat · Archived' }) === null).toBe(true);
  fireEvent.mouseDown(document.body);

  await f.menu('Show archived conversations');
  openSelect();
  expect(await f.view.findByRole('option', { name: 'Earlier archived chat · Archived' })).toBeTruthy();
  fireEvent.mouseDown(document.body);
  await f.menu('Restore conversation');
  await waitFor(() => expect(f.onArchive).toHaveBeenCalledTimes(2));
  expect(f.onArchive.mock.calls[1]).toEqual([targetId, false]);
  f.view.rerender(f.ui());
  expect(f.view.container.textContent).toContain('Target chat');
  expect(f.onContinue).not.toHaveBeenCalled();
  expect(f.onCreate).not.toHaveBeenCalled();
  expect(f.onSelect).not.toHaveBeenCalled();
  expect(f.error).not.toHaveBeenCalled();
});

test('delete explains its Session-only scope, cancellation sends nothing, and a host 409 preserves the dialog and selection', async () => {
  const f = fixture();
  f.onDelete.mockRejectedValue(new BackendHttpError({
    method: 'DELETE', path: `/api/conversations/${targetId}`, status: 409,
    body: { code: 'AGENT_EXECUTION_DELIVERY_PENDING', message: 'The delegated result is still pending.' },
  }));
  await f.menu('Delete');
  let dialog = await f.view.findByRole('dialog');
  expect(dialog.textContent).toContain('Companion memory and shared files are kept.');
  fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
  await waitFor(() => expect(f.view.queryByRole('dialog') === null).toBe(true));
  expect(f.onDelete).not.toHaveBeenCalled();

  await f.menu('Delete');
  dialog = await f.view.findByRole('dialog');
  fireEvent.click(within(dialog).getByRole('button', { name: 'OK' }));
  await waitFor(() => expect(f.error).toHaveBeenCalledTimes(1));
  expect(f.onDelete).toHaveBeenCalledTimes(1);
  expect(f.onDelete.mock.calls[0]).toEqual([targetId]);
  expect(f.view.getByRole('dialog')).toBe(dialog);
  expect(f.error.mock.calls[0][0]).toBe(nomi.cohabit.sessions.deliveryPending);
  expect(f.view.container.textContent).toContain('Target chat');
  expect(f.onContinue).not.toHaveBeenCalled();
  expect(f.onCreate).not.toHaveBeenCalled();
  expect(f.onSelect).not.toHaveBeenCalled();
  expect(f.success).not.toHaveBeenCalled();
});
