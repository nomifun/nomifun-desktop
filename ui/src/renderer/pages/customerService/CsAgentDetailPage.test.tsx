import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { MemoryRouter, Route, Routes, useNavigate, type NavigateFunction } from 'react-router-dom';
import { Message, Modal } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { ICsAgent, ICsHandoff, ICsNote } from '@/common/adapter/ipcBridge';
import { parseCsAgentId, parseCsDialogueId, parseCsHandoffId, parseCsNoteId, parseUserId } from '@/common/types/ids';
import * as excludedChannels from './CsChannelBotsSection';
import CsAgentDetailPage from './CsAgentDetailPage';

const i18n = createInstance();
await i18n.init({ lng: 'en', keySeparator: false, resources: { en: { translation: {
  'customerService.detail.save': 'Save identity', 'customerService.detail.delete': 'Delete agent',
  'customerService.notes.add': 'Add note', 'customerService.notes.contentPlaceholder': 'Note content',
  'customerService.notes.more': 'Note actions', 'customerService.notes.delete': 'Delete note',
  'customerService.handoffs.claim': 'Claim', 'customerService.handoffs.cancel': 'Cancel handoff',
  'customerService.handoffs.resolve': 'Resolve', 'customerService.handoffs.refresh': 'Refresh handoffs',
  'customerService.handoffs.resolutionPlaceholder': 'Resolution',
} } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(fn => fn()); });
function calls<T>() {
  const pending: Array<{ resolve: (value: T) => void; reject: (error: unknown) => void }> = [];
  return { pending, invoke: () => new Promise<T>((resolve, reject) => pending.push({ resolve, reject })) };
}
const uuid = (n: number) => '019b0000-0000-7000-8000-' + String(n).padStart(12, '0');
const agent = (n = 1): ICsAgent => ({
  cs_agent_id: parseCsAgentId(uuid(n)), name: 'Agent ' + n, greeting: '', persona: '', service_policy: '',
  provider_id: null, model: null, knowledge_base_ids: [], enabled: true, max_concurrent: 8,
  audit_retention_days: 30, created_at: 1, updated_at: 1,
});
const note = (content: string, n = 1): ICsNote => ({
  cs_note_id: parseCsNoteId(uuid(n)), cs_agent_id: agent().cs_agent_id, kind: 'faq', content,
  aliases: '', enabled: true, created_at: 1, updated_at: 1,
});
const handoff = (reason: string, status: ICsHandoff['status'] = 'pending'): ICsHandoff => ({
  cs_handoff_id: parseCsHandoffId(uuid(1)), cs_agent_id: agent().cs_agent_id,
  cs_dialogue_id: parseCsDialogueId(uuid(1)), requested_by: parseUserId(uuid(1)), updated_by: parseUserId(uuid(1)),
  idempotency_key: 'fixture', reason, summary: '', status, claimed_by: null, resolution: '', created_at: 1, updated_at: 1,
});
async function fixture() {
  const notes = calls<ICsNote[]>(); const handoffs = calls<ICsHandoff[]>();
  const patches = calls<ICsAgent>(); const creates = calls<ICsNote>(); const notePatches = calls<ICsNote>();
  const claims = calls<ICsHandoff>(); const cancels = calls<ICsHandoff>(); const resolves = calls<ICsHandoff>();
  const deletions = calls<unknown>();
  const get = spyOn(ipcBridge.customerService.getAgent, 'invoke').mockImplementation(async ({ cs_agent_id }) => cs_agent_id === agent(2).cs_agent_id ? agent(2) : agent());
  const listNotes = spyOn(ipcBridge.customerService.listNotes, 'invoke').mockImplementation(notes.invoke);
  const listHandoffs = spyOn(ipcBridge.customerService.listHandoffs, 'invoke').mockImplementation(handoffs.invoke);
  const patch = spyOn(ipcBridge.customerService.patchAgent, 'invoke').mockImplementation(patches.invoke);
  const create = spyOn(ipcBridge.customerService.createNote, 'invoke').mockImplementation(creates.invoke);
  const patchNote = spyOn(ipcBridge.customerService.patchNote, 'invoke').mockImplementation(notePatches.invoke);
  const removeNote = spyOn(ipcBridge.customerService.removeNote, 'invoke').mockResolvedValue(undefined);
  const removeAgent = spyOn(ipcBridge.customerService.removeAgent, 'invoke').mockImplementation(deletions.invoke);
  const claim = spyOn(ipcBridge.customerService.claimHandoff, 'invoke').mockImplementation(claims.invoke);
  const cancel = spyOn(ipcBridge.customerService.cancelHandoff, 'invoke').mockImplementation(cancels.invoke);
  const resolve = spyOn(ipcBridge.customerService.resolveHandoff, 'invoke').mockImplementation(resolves.invoke);
  const providers = spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]);
  const bases = spyOn(ipcBridge.knowledge.listBases, 'invoke').mockResolvedValue([]);
  // Excluded channel/plugin surface is inert, not audited or exercised.
  const channels = spyOn(excludedChannels, 'default').mockImplementation(() => null);
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  const confirmations: Array<Parameters<typeof Modal.confirm>[0]> = [];
  const closeConfirm = mock();
  const confirm = spyOn(Modal, 'confirm').mockImplementation(props => { confirmations.push(props); return { close: closeConfirm, update: mock() }; });
  for (const spy of [get, listNotes, listHandoffs, patch, create, patchNote, removeNote, removeAgent, claim, cancel, resolve, providers, bases, channels, success, error, confirm]) restore.push(() => spy.mockRestore());
  let navigate!: NavigateFunction;
  function Navigation() { navigate = useNavigate(); return null; }
  const view = render(<SWRConfig value={{ provider: () => new Map(), dedupingInterval: 0 }}><I18nextProvider i18n={i18n}>
    <MemoryRouter initialEntries={['/customer-service/' + agent().cs_agent_id]}><Navigation /><Routes>
      <Route path='/customer-service/:cs_agent_id' element={<CsAgentDetailPage />} />
      <Route path='/customer-service' element={<div>Agent list</div>} />
    </Routes></MemoryRouter>
  </I18nextProvider></SWRConfig>);
  await act(async () => {});
  return {
    view, notes: notes.pending, handoffs: handoffs.pending, patches: patches.pending, creates: creates.pending,
    notePatches: notePatches.pending, claims: claims.pending, cancels: cancels.pending, resolves: resolves.pending, deletions: deletions.pending,
    patch, create, patchNote, removeNote, removeAgent, listNotes, listHandoffs, claim, cancel, resolve, success, error, confirmations, closeConfirm,
    click: (name: string) => fireEvent.click(view.getByRole('button', { name })),
    submitModal: () => fireEvent.click(view.baseElement.querySelector('.arco-modal-footer .arco-btn-primary')!),
    switchTo: async (n: number) => { await act(async () => { void navigate('/customer-service/' + agent(n).cs_agent_id); }); },
    change: (input: HTMLElement, value: string) => fireEvent.change(input, { target: { value } }),
  };
}

test('unrelated PATCH and an identity save response cannot overwrite an edited draft', async () => {
  const f = await fixture();
  const name = f.view.getByDisplayValue('Agent 1');
  f.change(name, 'unsaved name');
  fireEvent.click(f.view.container.querySelector('.arco-switch')!);
  await act(async () => { f.patches[0]!.resolve({ ...agent(), enabled: false }); });
  expect((name as HTMLInputElement).value).toBe('unsaved name');
  f.click('Save identity');
  f.change(name, 'newer edit');
  await act(async () => { f.patches[1]!.resolve({ ...agent(), name: 'unsaved name' }); });
  expect((name as HTMLInputElement).value).toBe('newer edit');
});

test('identity save blocks same-render reentry and stays silent after navigation', async () => {
  const f = await fixture();
  act(() => { f.click('Save identity'); f.click('Save identity'); });
  expect(f.patch).toHaveBeenCalledTimes(1);
  await f.switchTo(2);
  await act(async () => { f.patches[0]!.resolve(agent()); });
  expect(f.success).not.toHaveBeenCalled();
  expect(f.view.getByDisplayValue('Agent 2')).not.toBeNull();
});

test('navigation resets the note dialog and ignores old note/handoff snapshots', async () => {
  const f = await fixture();
  f.click('Add note'); f.change(f.view.getByPlaceholderText('Note content'), 'old draft');
  await f.switchTo(2);
  expect(f.view.queryAllByPlaceholderText('Note content').length).toBe(0);
  await act(async () => { f.notes[1]!.resolve([note('current note')]); f.handoffs[1]!.resolve([handoff('current handoff')]); });
  await act(async () => { f.notes[0]!.resolve([note('obsolete note')]); f.handoffs[0]!.resolve([handoff('obsolete handoff')]); });
  expect(f.view.queryAllByText('obsolete note').length).toBe(0);
  expect(f.view.queryAllByText('obsolete handoff').length).toBe(0);
  expect(f.view.getByText('current note')).not.toBeNull();
});

test('only the latest handoff refresh controls rows, errors and loading', async () => {
  const f = await fixture();
  await act(async () => { f.handoffs[0]!.resolve([handoff('initial')]); });
  act(() => { f.click('Refresh handoffs'); f.click('Refresh handoffs'); });
  await act(async () => { f.handoffs[2]!.resolve([handoff('latest')]); });
  await act(async () => { f.handoffs[1]!.reject('old failure'); });
  expect(f.error).not.toHaveBeenCalled();
  expect(f.view.getByText('latest')).not.toBeNull();
});

test('a note mutation refresh supersedes the initial snapshot', async () => {
  const f = await fixture();
  f.click('Add note'); f.change(f.view.getByPlaceholderText('Note content'), 'new note');
  f.submitModal();
  await act(async () => { f.creates[0]!.resolve(note('new note')); });
  await act(async () => { f.notes[1]!.resolve([note('new note')]); });
  await act(async () => { f.notes[0]!.resolve([]); });
  expect(f.view.queryAllByText('new note').length).toBe(1);
});

test.each([false, true])('note save is single-flight and cannot affect a new page (failure=%s)', async failure => {
  const f = await fixture();
  f.click('Add note'); f.change(f.view.getByPlaceholderText('Note content'), 'note A');
  act(() => { f.submitModal(); f.submitModal(); });
  expect(f.create).toHaveBeenCalledTimes(1);
  await f.switchTo(2);
  f.click('Add note'); f.change(f.view.getByPlaceholderText('Note content'), 'note B');
  await act(async () => { if (failure) f.creates[0]!.reject('late failure'); else f.creates[0]!.resolve(note('note A')); });
  expect((f.view.getByPlaceholderText('Note content') as HTMLTextAreaElement).value).toBe('note B');
  expect(f.success).not.toHaveBeenCalled(); expect(f.error).not.toHaveBeenCalled();
  expect(f.listNotes).toHaveBeenCalledTimes(2);
});

test.each(['claim', 'cancel', 'resolve'] as const)('handoff %s is single-flight and its late result is page-owned', async action => {
  const f = await fixture();
  await act(async () => { f.handoffs[0]!.resolve([handoff('request', action === 'resolve' ? 'claimed' : 'pending')]); });
  if (action === 'resolve') {
    f.click('Resolve'); f.change(f.view.getByPlaceholderText('Resolution'), 'completed');
    act(() => { f.submitModal(); f.submitModal(); });
  } else act(() => { f.click(action === 'claim' ? 'Claim' : 'Cancel handoff'); f.click(action === 'claim' ? 'Claim' : 'Cancel handoff'); });
  expect(f[action]).toHaveBeenCalledTimes(1);
  await f.switchTo(2);
  await act(async () => { f[action === 'claim' ? 'claims' : action === 'cancel' ? 'cancels' : 'resolves'][0]!.resolve(handoff('old result')); });
  expect(f.success).not.toHaveBeenCalled();
  expect(f.listHandoffs).toHaveBeenCalledTimes(2);
  expect(f.view.queryAllByPlaceholderText('Resolution').length).toBe(0);
});

test('note delete confirmation is closed and its retained callback is inert after navigation', async () => {
  const f = await fixture();
  await act(async () => { f.notes[0]!.resolve([note('remove me')]); });
  f.click('Note actions');
  fireEvent.click(await f.view.findByText('Delete note'));
  expect(f.confirmations.length).toBe(1);
  await f.switchTo(2);
  act(() => { void f.confirmations[0]!.onOk?.(); });
  expect(f.removeNote).not.toHaveBeenCalled();
  expect(f.closeConfirm).toHaveBeenCalledTimes(1);
});

test('a settings PATCH failure is reported without losing the identity draft', async () => {
  const f = await fixture();
  f.change(f.view.getByDisplayValue('Agent 1'), 'keep my draft');
  fireEvent.click(f.view.container.querySelector('.arco-switch')!);
  await act(async () => { f.patches[0]!.reject('settings offline'); });
  expect(f.error).toHaveBeenCalledWith('settings offline');
  expect(f.view.getByDisplayValue('keep my draft')).not.toBeNull();
});

test('a completed agent deletion cannot navigate away from the next detail page', async () => {
  const f = await fixture();
  f.click('Delete agent');
  const confirm = f.view.baseElement.querySelector('.arco-popconfirm .arco-btn-primary');
  expect(confirm).not.toBeNull();
  fireEvent.click(confirm!);
  expect(f.removeAgent).toHaveBeenCalledTimes(1);
  await f.switchTo(2);
  await act(async () => { f.deletions[0]!.resolve(undefined); });
  expect(f.success).not.toHaveBeenCalled();
  expect(f.view.queryByText('Agent list')).toBeNull();
  expect(f.view.getByDisplayValue('Agent 2')).not.toBeNull();
});
