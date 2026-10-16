/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { ICompanionMemory } from '@/common/adapter/ipcBridge';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { parseCompanionId, parseCompanionMemoryId } from '@/common/types/ids';
import MergeAssistantPane from './MergeAssistantPane';

const companionId = parseCompanionId('019b0000-0000-7000-8000-000000000071');
const first: ICompanionMemory = { memory_id: parseCompanionMemoryId('019b0000-0000-7000-8000-000000000072'), companion_id: companionId,
  revision: 2, kind: 'knowledge', content: 'First fact', tags: [], importance: 0.5, strength: 0.8, pinned: false, source: 'manual',
  status: 'active', created_at: 1, updated_at: 1, last_reinforced_at: 1 };
const second = { ...first, memory_id: parseCompanionMemoryId('019b0000-0000-7000-8000-000000000073'), revision: 5, content: 'Second fact' };
const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'nomi.memory.conflictDraftKept': 'New facts exist; your draft is preserved.', 'nomi.memory.refreshMerge': 'Reload suggestions',
  'nomi.memories.mergeSubmit': 'Merge selected', 'nomi.memory.conflict': 'Refresh before retrying',
} } } });
const restores: Array<() => void> = [];
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });

test('a concurrent source update rejects the observed merge versions, preserves the draft and requires explicit reload', async () => {
  let current = [first, second];
  const suggestions = spyOn(ipcBridge.companion.memoryMergeSuggestions, 'invoke').mockImplementation(async () => [{ memories: current }]);
  const merge = spyOn(ipcBridge.companion.mergeMemories, 'invoke').mockImplementation(async request => {
    if (current.some(memory => request.expected_revisions[memory.memory_id] !== memory.revision)) {
      throw new BackendHttpError({ method: 'POST', path: '/merge', status: 409, body: { error: 'concurrent change' } });
    }
    return { ...first, content: request.merged_content, revision: 1 };
  });
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  for (const spy of [suggestions, merge, error, success]) restores.push(() => spy.mockRestore());
  const view = render(<I18nextProvider i18n={i18n}><MergeAssistantPane companionId={companionId} onMerged={() => {}} /></I18nextProvider>);
  const draft = await view.findByRole('textbox');
  fireEvent.change(draft, { target: { value: 'User merged draft' } });
  current = [{ ...first, revision: 3, content: 'Concurrent new fact' }, second];
  fireEvent.click(view.getByRole('button', { name: 'Merge selected' }));
  await waitFor(() => expect(view.getByRole('alert').textContent).toContain('your draft is preserved'));
  expect(merge.mock.calls[0][0].expected_revisions).toEqual({ [first.memory_id]: 2, [second.memory_id]: 5 });
  expect((view.getByRole('textbox') as HTMLTextAreaElement).value).toBe('User merged draft');
  fireEvent.click(view.getByRole('button', { name: 'Merge selected' }));
  await act(async () => {});
  expect(merge).toHaveBeenCalledTimes(1);
  expect(success).not.toHaveBeenCalled();
  suggestions.mockRejectedValueOnce(new Error('read unavailable'));
  fireEvent.click(view.getByRole('button', { name: 'Reload suggestions' }));
  await waitFor(() => expect(suggestions).toHaveBeenCalledTimes(2));
  await waitFor(() => expect((view.getByRole('textbox') as HTMLTextAreaElement).value).toBe('User merged draft'));
  fireEvent.click(view.getByRole('button', { name: 'Reload suggestions' }));
  await waitFor(() => expect(suggestions).toHaveBeenCalledTimes(3));
  await waitFor(() => expect(view.getAllByText('Concurrent new fact').length).toBeGreaterThan(0));
  fireEvent.click(view.getByRole('button', { name: 'Merge selected' }));
  await waitFor(() => expect(merge).toHaveBeenCalledTimes(2));
  expect(merge.mock.calls[1][0].expected_revisions).toEqual({ [first.memory_id]: 3, [second.memory_id]: 5 });
});
