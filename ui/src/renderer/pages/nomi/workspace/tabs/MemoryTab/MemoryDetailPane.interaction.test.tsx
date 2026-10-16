/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { ICompanionMemory } from '@/common/adapter/ipcBridge';
import { parseCompanionId, parseCompanionMemoryId } from '@/common/types/ids';
import MemoryDetailPane from './MemoryDetailPane';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'nomi.memory.content': 'Memory content',
  'nomi.memory.save': 'Save',
  'nomi.memory.loadLatest': 'Load latest',
  'nomi.memory.conflictDraftKept': 'This memory changed. Your draft has been kept.',
  'nomi.memory.conflict': 'The memory revision changed.',
  'nomi.memory.saveFailed': 'Could not save the memory.',
  'nomi.memories.saved': 'Memory saved',
  'nomi.memory.revision': 'Revision {{revision}}',
} } } });

const restores: Array<() => void> = [];
afterEach(() => {
  cleanup();
  restores.splice(0).reverse().forEach(restore => restore());
});

function muteToasts() {
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  restores.push(() => success.mockRestore(), () => error.mockRestore());
  return { success, error };
}

function memoryFixture(): ICompanionMemory {
  return {
    memory_id: parseCompanionMemoryId('019b0000-0000-7000-8000-000000000001'),
    companion_id: parseCompanionId('019b0000-0000-7000-8000-000000000002'),
    revision: 7,
    content: 'The original observed memory',
    kind: 'knowledge',
    tags: [],
    importance: 0.7,
    strength: 0.8,
    pinned: false,
    source: 'manual',
    status: 'active',
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    last_reinforced_at: 1_700_000_000_000,
  };
}

/** An in-memory CAS double with individually released requests, not a live backend. */
function memoryCas(initial: ICompanionMemory) {
  let current = { ...initial };
  const requests: Array<{ release: () => void }> = [];
  const mutate = async (patch: Partial<ICompanionMemory>, observed: ICompanionMemory): Promise<ICompanionMemory> => {
    await new Promise<void>(resolve => { requests.push({ release: resolve }); });
    if (observed.memory_id !== current.memory_id || observed.revision !== current.revision) {
      throw new BackendHttpError({
        method: 'PUT',
        path: `/api/companion/memories/${observed.memory_id}`,
        status: 409,
        body: { code: 'MEMORY_REVISION_CONFLICT', message: 'The observed memory revision is stale.' },
      });
    }
    current = { ...current, ...patch, revision: current.revision + 1, updated_at: current.updated_at + 1 };
    return { ...current };
  };
  const save = mock((content: string, observed: ICompanionMemory) => mutate({ content }, observed));
  const pin = mock((pinned: boolean, observed: ICompanionMemory) => mutate({ pinned }, observed));
  return {
    save,
    pin,
    get row() { return { ...current }; },
    externalEdit(content: string) {
      current = { ...current, content, revision: current.revision + 1, updated_at: current.updated_at + 1 };
      return { ...current };
    },
    async release(index: number) {
      await act(async () => { requests[index].release(); });
    },
  };
}

type PaneProps = React.ComponentProps<typeof MemoryDetailPane>;
function pane(
  memory: ICompanionMemory,
  onSave: PaneProps['onSave'],
  onRefresh: () => Promise<void>,
  actions: Partial<Pick<PaneProps, 'onTogglePin' | 'onToggleArchive'>> = {},
) {
  return <MemoryRouter><I18nextProvider i18n={i18n}>
    <MemoryDetailPane
      memory={memory}
      onSave={onSave}
      onRefresh={onRefresh}
      onTogglePin={async () => memory}
      onToggleArchive={async () => memory}
      onDelete={() => {}}
      {...actions}
    />
  </I18nextProvider></MemoryRouter>;
}

test('a live revision preserves a dirty draft and blocks saving until the user explicitly loads it', async () => {
  const toast = muteToasts();
  const original = memoryFixture();
  const backend = memoryCas(original);
  const refresh = mock(async () => {});
  const view = render(pane(original, backend.save, refresh));
  const query = within(view.container);
  const input = query.getByRole('textbox', { name: 'Memory content' }) as HTMLTextAreaElement;
  const save = query.getByRole('button', { name: 'Save' }) as HTMLButtonElement;

  fireEvent.change(input, { target: { value: 'My unsaved local correction' } });
  const live = backend.externalEdit('A concurrent correction from the other editor');
  view.rerender(pane(live, backend.save, refresh));

  expect(input.value).toBe('My unsaved local correction');
  expect(query.getByRole('alert').textContent).toContain(live.content);
  expect(save.disabled).toBe(true);
  fireEvent.click(save);
  expect(backend.save).not.toHaveBeenCalled();
  expect(refresh).not.toHaveBeenCalled();

  fireEvent.click(query.getByRole('button', { name: 'Load latest' }));
  expect(input.value).toBe(live.content);
  expect(refresh).toHaveBeenCalledTimes(1);
  expect(query.queryByRole('alert')).toBeNull();
  fireEvent.change(input, { target: { value: 'An explicit edit of the latest memory' } });
  fireEvent.click(save);
  expect(backend.save).toHaveBeenCalledTimes(1);
  expect(backend.save.mock.calls[0][1].revision).toBe(live.revision);
  expect(input.disabled).toBe(true);
  expect(backend.row).toEqual(live);

  await backend.release(0);
  await waitFor(() => expect(input.disabled).toBe(false));
  expect(backend.row.content).toBe('An explicit edit of the latest memory');
  expect(backend.row.revision).toBe(live.revision + 1);
  expect(toast.success).toHaveBeenCalledTimes(1);
  expect(toast.error).not.toHaveBeenCalled();
});

test('two editors submitting the same observed revision get one CAS winner and retain the losing draft without retry', async () => {
  const toast = muteToasts();
  const original = memoryFixture();
  const backend = memoryCas(original);
  const firstRefresh = mock(async () => {});
  const secondRefresh = mock(async () => {});
  const first = render(pane(original, backend.save, firstRefresh));
  const second = render(pane(original, backend.save, secondRefresh));
  const firstQuery = within(first.container);
  const secondQuery = within(second.container);
  const firstInput = firstQuery.getByRole('textbox', { name: 'Memory content' }) as HTMLTextAreaElement;
  const secondInput = secondQuery.getByRole('textbox', { name: 'Memory content' }) as HTMLTextAreaElement;
  const firstSave = firstQuery.getByRole('button', { name: 'Save' }) as HTMLButtonElement;
  const secondSave = secondQuery.getByRole('button', { name: 'Save' }) as HTMLButtonElement;

  fireEvent.change(firstInput, { target: { value: 'The first editor correction' } });
  fireEvent.change(secondInput, { target: { value: 'The second editor unsaved correction' } });
  fireEvent.click(firstSave);
  fireEvent.click(secondSave);
  expect(backend.save).toHaveBeenCalledTimes(2);
  expect(backend.save.mock.calls.map(call => call[1].revision)).toEqual([original.revision, original.revision]);
  expect(firstInput.disabled).toBe(true);
  expect(secondInput.disabled).toBe(true);

  await backend.release(0);
  await waitFor(() => expect(firstInput.disabled).toBe(false));
  const winner = backend.row;
  expect(winner.content).toBe('The first editor correction');
  expect(winner.revision).toBe(original.revision + 1);
  await backend.release(1);
  await waitFor(() => expect(secondQuery.getByRole('alert').textContent).toContain('Your draft has been kept.'));

  expect(secondInput.value).toBe('The second editor unsaved correction');
  expect(secondInput.disabled).toBe(false);
  expect(secondSave.disabled).toBe(true);
  expect(backend.row).toEqual(winner);
  expect(toast.error).toHaveBeenCalledTimes(1);
  expect(toast.error.mock.calls[0][0]).toBe('The memory revision changed.');
  expect(toast.success).toHaveBeenCalledTimes(1);
  expect(secondRefresh).not.toHaveBeenCalled();

  second.rerender(pane(winner, backend.save, secondRefresh));
  expect(secondInput.value).toBe('The second editor unsaved correction');
  expect(secondQuery.getByRole('alert').textContent).toContain(winner.content);
  fireEvent.click(secondSave);
  expect(backend.save).toHaveBeenCalledTimes(2);
  expect(backend.row).toEqual(winner);
});

test('a successful response supplies the next editing revision even when the parent sends an older row again', async () => {
  const toast = muteToasts();
  const original = memoryFixture();
  const backend = memoryCas(original);
  const refresh = mock(async () => {});
  const view = render(pane(original, backend.save, refresh));
  const query = within(view.container);
  const input = query.getByRole('textbox', { name: 'Memory content' }) as HTMLTextAreaElement;
  const save = query.getByRole('button', { name: 'Save' }) as HTMLButtonElement;

  fireEvent.change(input, { target: { value: 'The first committed content' } });
  fireEvent.click(save);
  await backend.release(0);
  await waitFor(() => expect(input.disabled).toBe(false));
  const committed = backend.row;
  view.rerender(pane({ ...original }, backend.save, refresh));
  expect(input.value).toBe(committed.content);
  expect(save.disabled).toBe(true);
  expect(query.queryByRole('alert')).toBeNull();

  fireEvent.change(input, { target: { value: 'A second edit based on the committed content' } });
  fireEvent.click(save);
  expect(backend.save).toHaveBeenCalledTimes(2);
  expect(backend.save.mock.calls[1][1].revision).toBe(committed.revision);
  expect(backend.save.mock.calls[1][1].content).toBe(committed.content);
  await backend.release(1);
  await waitFor(() => expect(input.disabled).toBe(false));
  view.rerender(pane({ ...original }, backend.save, refresh));

  expect(backend.row.revision).toBe(original.revision + 2);
  expect(input.value).toBe('A second edit based on the committed content');
  expect(save.disabled).toBe(true);
  expect(toast.success).toHaveBeenCalledTimes(2);
  expect(toast.error).not.toHaveBeenCalled();
  expect(refresh).not.toHaveBeenCalled();
});

test('a successful pin advances the observed revision without discarding an unsaved content draft', async () => {
  const toast = muteToasts();
  const original = memoryFixture();
  const backend = memoryCas(original);
  const refresh = mock(async () => {});
  const actions = { onTogglePin: (pinned: boolean) => backend.pin(pinned, original) };
  const view = render(pane(original, backend.save, refresh, actions));
  const query = within(view.container);
  const input = query.getByRole('textbox', { name: 'Memory content' }) as HTMLTextAreaElement;
  const save = query.getByRole('button', { name: 'Save' }) as HTMLButtonElement;

  fireEvent.change(input, { target: { value: 'My content draft while pinning this memory' } });
  fireEvent.click(query.getAllByRole('switch')[0]);
  expect(backend.pin).toHaveBeenCalledTimes(1);
  expect(backend.pin.mock.calls[0][1].revision).toBe(original.revision);
  expect(input.disabled).toBe(true);
  await backend.release(0);
  await waitFor(() => expect(input.disabled).toBe(false));

  const pinned = backend.row;
  expect(pinned.pinned).toBe(true);
  expect(pinned.content).toBe(original.content);
  expect(pinned.revision).toBe(original.revision + 1);
  view.rerender(pane(pinned, backend.save, refresh, actions));
  expect(input.value).toBe('My content draft while pinning this memory');
  expect(query.queryByRole('alert')).toBeNull();
  expect(save.disabled).toBe(false);

  fireEvent.click(save);
  expect(backend.save).toHaveBeenCalledTimes(1);
  expect(backend.save.mock.calls[0][1]).toEqual(pinned);
  await backend.release(1);
  await waitFor(() => expect(input.disabled).toBe(false));
  expect(backend.row.content).toBe('My content draft while pinning this memory');
  expect(backend.row.revision).toBe(pinned.revision + 1);
  expect(backend.row.pinned).toBe(true);
  expect(toast.success).toHaveBeenCalledTimes(1);
  expect(toast.error).not.toHaveBeenCalled();
  expect(refresh).not.toHaveBeenCalled();
});
