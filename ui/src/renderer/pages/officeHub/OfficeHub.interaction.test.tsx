import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { officeHub } from '@/common/adapter/officeHubBridge';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { AuthorizationFlow, ExecuteRequest, OfficeItem, OfficeOperation, OfficeQueryResult, OfficeStatus } from '@/common/types/officeHub';
import office from '@/renderer/services/i18n/locales/en-US/office.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import OfficeHubPage from './index';

const locale = createInstance();
const restores: Array<() => void> = [];
const track = <T extends { mockRestore(): void }>(spy: T) => { restores.push(() => spy.mockRestore()); return spy; };
const actor = { app_id: 'real-app', user_id: 'real-user', name: 'Verified user' };
const empty = (): OfficeQueryResult => ({ items: [], fields: [], has_more: false, next_cursor: null, warnings: [] });
function status(): OfficeStatus {
  return { provider: 'feishu', connection: { id: 'connection', name: 'Work connection', enabled: true, profile: 'work', executable: null, created_at: '2026-10-26T00:00:00Z', updated_at: '2026-10-26T00:00:00Z' }, runtime: { phase: 'ready', executable: 'C:/tools/lark-cli.exe', version: '1.0.98', source: 'external', message: null, official_install_command: 'npx @larksuite/cli@latest install' }, phase: 'ready', actor: { ...actor }, host_name: 'backend-machine', host_os: 'windows', message: null, permissions: ['documents.browse','documents.create','documents.delete','people.read','tasks.read','tasks.write','tasklists.read','calendar.read','calendar.write','base.read','base.write'].map(capability => ({ capability, granted: true, required_scopes: [capability + '-scope'] })) };
}
function item(id: string, title: string): OfficeItem {
  return { id, title, kind: 'task', url: null, description: null, modified_at: null, opened_at: null, assignee_ids: [actor.user_id], creator_id: actor.user_id, tasklist_ids: [], due: null, completed: false, start: null, end: null, calendar_id: null, base_token: null, table_id: null, pending_operation_id: null, pending_status: null, fields: {} };
}
function receipt(request: ExecuteRequest, state: OfficeOperation['status'] = 'succeeded'): OfficeOperation {
  return { id: 'original-operation', request_id: request.request_id, connection_id: 'connection', actor: request.expected_actor, profile: request.profile, status: state, created_at: '2026-10-26T00:00:00Z', updated_at: '2026-10-26T00:00:00Z', items: request.items.map((mutation, index) => ({ id: 'item-' + index, mutation, target_key: 'target-' + index, status: state === 'partial' ? index ? 'unknown' : 'succeeded' : state, remote_id: null, remote_url: null, message: null, attempts: [], observations: [] })) };
}
function flow(phase: AuthorizationFlow['phase'] = 'waiting_user'): AuthorizationFlow {
  return { id: '0190f5fe-7c00-7a00-8000-000000000901', kind: 'login', phase, verification_url: 'https://open.feishu.cn/auth', expires_at: null, missing_scopes: [], console_url: null, message: null };
}
beforeAll(async () => { await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { office, common } } } }); });
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });
function fixture(initial = status()) {
  let current = initial;
  const success = track(spyOn(Message, 'success').mockReturnValue(() => {}));
  const getStatus = track(spyOn(officeHub, 'status').mockImplementation(async () => structuredClone(current)));
  const profiles = track(spyOn(officeHub, 'profiles').mockResolvedValue([{ name: 'work', app_id: actor.app_id, brand: 'feishu', user_name: actor.name, token_status: 'valid' }]));
  const query = track(spyOn(officeHub, 'query').mockImplementation(async () => empty()));
  const external = track(spyOn(ipcBridge.shell.openExternal, 'invoke').mockResolvedValue(undefined));
  const remember = track(spyOn(officeHub, 'rememberResource').mockResolvedValue(undefined));
  const history = track(spyOn(officeHub, 'operations').mockResolvedValue({ items: [], next_cursor: null }));
  const prepare = track(spyOn(officeHub, 'prepare').mockResolvedValue(initial.runtime));
  const execute = track(spyOn(officeHub, 'execute').mockImplementation(async request => receipt(request)));
  const request = track(spyOn(officeHub, 'request').mockRejectedValue(new BackendHttpError({ method: 'GET', path: '/api/office-hub/operations/request/test', status: 404, body: { error: 'Not found' } })));
  const reconcile = track(spyOn(officeHub, 'reconcile').mockImplementation(async () => { throw new Error('Unexpected reconciliation'); }));
  const open = track(spyOn(officeHub, 'operation').mockImplementation(async () => { throw new Error('Missing operation'); }));
  const auth = track(spyOn(officeHub, 'auth').mockResolvedValue(flow()));
  const startAuth = track(spyOn(officeHub, 'startAuth').mockResolvedValue(flow()));
  const cancelAuth = track(spyOn(officeHub, 'cancelAuth').mockResolvedValue(flow('cancelling')));
  const updateConnection = track(spyOn(officeHub, 'updateConnection').mockImplementation(async body => { current = { ...current, phase: 'ready', connection: { ...current.connection, ...body } }; return current.connection; }));
  const mount = async (path = '/office') => {
    const router = createMemoryRouter([{ path: '/office', element: <OfficeHubPage /> }], { initialEntries: [path] });
    restores.push(() => router.dispose());
    let view!: ReturnType<typeof render>;
    await act(async () => { view = render(<I18nextProvider i18n={locale}><RouterProvider router={router} /></I18nextProvider>); });
    await waitFor(() => expect(getStatus).toHaveBeenCalled());
    return view;
  };
  return { mount, external, remember, success, getStatus, profiles, query, history, prepare, execute, request, reconcile, open, auth, startAuth, cancelAuth, updateConnection, setStatus: (next: OfficeStatus) => { current = next; } };
}
async function pressReady(view: ReturnType<typeof render>, name: string) {
  const button = await view.findByRole('button', { name });
  await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
  await act(async () => { fireEvent.click(button); });
}
const grant = (source: OfficeStatus, capability: string, value: boolean) => { source.permissions.find(permission => permission.capability === capability)!.granted = value; };

describe('compact Office Hub with isolated real API boundaries', () => {
  test('document names open their exact browser link without an internal content query', async () => {
    const f = fixture(); const doc = { ...item('doc_real', 'Cloud document'), kind: 'docx', url: 'https://team.feishu.cn/docx/doc_real?from=search' };
    f.query.mockImplementation(async query => query.kind === 'documents' ? { ...empty(), items: [doc] } : empty());
    const view = await f.mount('/office?tab=documents&document=retired-internal-route');
    expect(f.external).not.toHaveBeenCalled();
    fireEvent.click(await view.findByRole('button', { name: 'Cloud document' }));
    await waitFor(() => expect(f.external).toHaveBeenCalledWith(doc.url));
    expect(f.remember).toHaveBeenCalledWith({ url: doc.url, title: doc.title });
    expect(view.queryByRole('article')).toBeNull(); expect(view.queryByRole('textbox', { name: /paragraph/i })).toBeNull();
    expect(f.query.mock.calls.every(([query]) => query.kind === 'documents')).toBe(true); expect(f.execute).not.toHaveBeenCalled();
  });
  test('Base has one document filter and browser entry, with no duplicate top-level tab', async () => {
    const f = fixture(); const base = { ...item('base_real', 'Cloud Base'), kind: 'base', url: 'https://team.feishu.cn/base/base_real?table=tbl_real', base_token: 'base_real', table_id: 'tbl_real' };
    f.query.mockResolvedValue({ ...empty(), items: [base] });
    const view = await f.mount('/office?tab=documents');
    expect(view.queryByRole('tab', { name: office.resources.filters.base })).toBeNull();
    fireEvent.click(view.getByRole('button', { name: office.resources.filters.base }));
    await waitFor(() => expect(f.query.mock.calls.at(-1)![0]).toMatchObject({ kind: 'documents', filter: 'base' }));
    fireEvent.click(await view.findByRole('button', { name: 'Cloud Base' }));
    await waitFor(() => expect(f.external).toHaveBeenCalledWith(base.url));
    expect(f.query.mock.calls.some(([query]) => query.kind === 'base_records')).toBe(false);
    expect(view.queryByRole('button', { name: 'Delete document: Cloud Base' })).toBeNull();
  });
  test('pasted links validate before opening and wiki content stays in the browser', async () => {
    const f = fixture(), view = await f.mount('/office?tab=documents');
    fireEvent.click(view.getAllByRole('button', { name: office.resources.openLink })[0]);
    fireEvent.change(view.getByLabelText(office.resources.link), { target: { value: 'https://evil.test/docx/token' } });
    fireEvent.click(view.getByRole('button', { name: office.actions.openBrowser }));
    await view.findByText(office.resources.invalidLink); expect(f.external).not.toHaveBeenCalled();
    const url = 'https://team.feishu.cn/wiki/wiki_real?from=link';
    fireEvent.change(view.getByLabelText(office.resources.link), { target: { value: url } });
    fireEvent.click(view.getByRole('button', { name: office.actions.openBrowser }));
    await waitFor(() => expect(f.external).toHaveBeenCalledWith(url));
    expect(view.queryByRole('dialog')).toBeNull(); expect(f.execute).not.toHaveBeenCalled();
  });
  test('delete confirms one exact resource and cancellation sends nothing', async () => {
    const f = fixture(); const doc = { ...item('doc_real', 'Delete this document'), kind: 'docx', url: 'https://team.feishu.cn/docx/doc_real' };
    f.query.mockImplementation(async query => query.kind === 'documents' ? { ...empty(), items: [doc] } : empty());
    const view = await f.mount('/office?tab=documents');
    fireEvent.click(await view.findByRole('button', { name: 'Delete document: Delete this document' }));
    let modal = within(view.getByRole('dialog', { name: office.documents.deleteTitle }));
    expect(modal.getByText(doc.title)).toBeTruthy(); expect(modal.getByText(doc.url)).toBeTruthy(); expect(f.execute).not.toHaveBeenCalled();
    fireEvent.click(modal.getByRole('button', { name: office.actions.cancel })); expect(f.execute).not.toHaveBeenCalled();
    fireEvent.click(await view.findByRole('button', { name: 'Delete document: Delete this document' }));
    modal = within(view.getByRole('dialog')); fireEvent.click(modal.getByRole('button', { name: office.actions.delete }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
    expect(f.execute.mock.calls[0][0].items[0]).toMatchObject({ kind: 'document_delete', document_id: 'doc_real', document_type: 'docx', title: doc.title });
    expect(f.execute.mock.calls[0][0].expected_actor).toEqual(actor); expect(f.execute.mock.calls[0][0].profile).toBe('work');
    await waitFor(() => expect(f.success).toHaveBeenCalledWith({ content: office.deleted, duration: 3000 }));
  });
  test('delete authorization returns to the same target and waits for explicit confirmation', async () => {
    const s = status(); grant(s, 'documents.delete', false); const f = fixture(s);
    const doc = { ...item('doc_real', 'Retained deletion target'), kind: 'docx', url: 'https://team.feishu.cn/docx/doc_real' };
    f.query.mockResolvedValue({ ...empty(), items: [doc] });
    f.startAuth.mockImplementation(async () => { const next = structuredClone(s); grant(next, 'documents.delete', true); f.setStatus(next); return flow('complete'); });
    const view = await f.mount('/office?tab=documents'); fireEvent.click(await view.findByRole('button', { name: 'Delete document: Retained deletion target' }));
    fireEvent.click(within(view.getByRole('dialog')).getByRole('button', { name: office.actions.delete }));
    await view.findByText(office.auth.phase.complete); expect(f.execute).not.toHaveBeenCalled();
    expect(f.startAuth.mock.calls[0][0].scopes).toEqual(['documents.delete-scope']);
    fireEvent.click(view.getByRole('button', { name: office.auth.return }));
    expect(within(view.getByRole('dialog')).getByText(doc.title)).toBeTruthy(); expect(f.execute).not.toHaveBeenCalled();
    fireEvent.click(within(view.getByRole('dialog')).getByRole('button', { name: office.actions.delete }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
  });
  test('unresolved deletion stays locked and refreshed pending rows point to the original operation', async () => {
    const f = fixture(); const doc = { ...item('doc_real', 'Pending document'), kind: 'docx', url: 'https://team.feishu.cn/docx/doc_real' };
    f.query.mockImplementation(async () => ({ ...empty(), items: [doc] }));
    f.execute.mockImplementation(async request => { doc.pending_operation_id = 'original-operation'; doc.pending_status = 'unknown'; return receipt(request, 'unknown'); });
    const view = await f.mount('/office?tab=documents'); fireEvent.click(await view.findByRole('button', { name: 'Delete document: Pending document' }));
    fireEvent.click(within(view.getByRole('dialog')).getByRole('button', { name: office.actions.delete }));
    await waitFor(() => expect((within(view.getByRole('dialog')).getByRole('button', { name: office.actions.delete }) as HTMLButtonElement).disabled).toBe(true));
    fireEvent.click(within(view.getByRole('dialog')).getByRole('button', { name: office.actions.delete })); expect(f.execute).toHaveBeenCalledTimes(1);
    fireEvent.click(within(view.getByRole('dialog')).getByRole('button', { name: office.actions.cancel }));
    await waitFor(() => expect((view.getByRole('button', { name: 'Delete document: Pending document' }) as HTMLButtonElement).disabled).toBe(true));
    expect(view.getByRole('button', { name: office.actions.original })).toBeTruthy(); expect(f.execute).toHaveBeenCalledTimes(1);
  });
  test('existing documents browse and paginate without a keyword, then search resets the cursor', async () => {
    const f = fixture();
    f.query.mockImplementation(async query => query.kind === 'documents' ? { ...empty(), items: [{ ...item('doc_real', query.query ? 'Search result' : query.cursor ? 'Another existing document' : 'Existing Wiki document'), kind: 'wiki', url: 'https://team.feishu.cn/wiki/wiki_real' }], has_more: true, next_cursor: query.query ? 'search-next' : query.cursor ? 'list-third' : 'list-next' } : empty());
    const view = await f.mount('/office?tab=documents');
    await view.findByRole('button', { name: 'Existing Wiki document' });
    expect(f.query.mock.calls[0][0]).toEqual({ kind: 'documents', query: '', filter: 'all', cursor: null });
    expect(f.startAuth).not.toHaveBeenCalled();
    fireEvent.click(view.getByRole('button', { name: office.actions.more }));
    await view.findByRole('button', { name: 'Another existing document' });
    expect(f.query.mock.calls.some(([query]) => query.kind === 'documents' && query.query === '' && query.cursor === 'list-next')).toBe(true);
    fireEvent.change(view.getByLabelText(office.resources.search), { target: { value: 'Report' } });
    fireEvent.submit(view.getByLabelText(office.resources.search).closest('form')!);
    await view.findByRole('button', { name: 'Search result' });
    expect(f.query.mock.calls.some(([query]) => query.kind === 'documents' && query.query === 'Report' && query.cursor === null)).toBe(true);
    fireEvent.click(view.getByRole('button', { name: office.actions.more }));
    await waitFor(() => expect(f.query.mock.calls.some(([query]) => query.kind === 'documents' && query.cursor === 'search-next')).toBe(true));
  });
  test('type filters query the complete server scope and reset page state', async () => {
    const f = fixture();
    f.query.mockImplementation(async query => query.kind === 'documents' ? { ...empty(), items: [{ ...item('resource', query.filter === 'base' ? 'Existing Base outside the first page' : 'Existing document'), kind: query.filter === 'base' ? 'base' : 'docx', url: 'https://team.feishu.cn/docx/doc_real' }], has_more: true, next_cursor: query.cursor ? 'third' : 'second' } : empty());
    const view = await f.mount('/office?tab=documents'); await view.findByRole('button', { name: 'Existing document' });
    fireEvent.click(view.getByRole('button', { name: office.actions.more }));
    await waitFor(() => expect(f.query.mock.calls.some(([query]) => query.kind === 'documents' && query.cursor === 'second')).toBe(true));
    fireEvent.click(view.getByRole('button', { name: office.resources.filters.base }));
    await view.findByRole('button', { name: 'Existing Base outside the first page' });
    expect(f.query.mock.calls.at(-1)![0]).toEqual({ kind: 'documents', query: '', filter: 'base', cursor: null });
    expect((view.getByRole('button', { name: office.actions.previous }) as HTMLButtonElement).disabled).toBe(true);
  });
  test('a failed list query stays an error and explicit retry never claims the account is empty', async () => {
    const f = fixture(); let fail = true;
    f.query.mockImplementation(async query => { if (query.kind !== 'documents') return empty(); if (fail) throw new Error('Search index unavailable'); return { ...empty(), items: [{ ...item('doc', 'Recovered document'), kind: 'docx', url: 'https://team.feishu.cn/docx/doc' }] }; });
    const view = await f.mount('/office?tab=documents'); await view.findByText('Search index unavailable');
    expect(view.queryByText(office.resources.queryEmpty)).toBeNull();
    fail = false; fireEvent.click(view.getByRole('button', { name: office.actions.check }));
    await view.findByRole('button', { name: 'Recovered document' }); expect(f.execute).not.toHaveBeenCalled();
  });
  test('switching the connected identity cannot reuse the previous accounts page cursor', async () => {
    const f = fixture();
    f.query.mockResolvedValue({ ...empty(), items: [{ ...item('doc', 'Existing document'), kind: 'docx', url: 'https://team.feishu.cn/docx/doc' }], has_more: true, next_cursor: 'second-page' });
    const view = await f.mount('/office?tab=documents'); await view.findByRole('button', { name: 'Existing document' });
    fireEvent.click(view.getByRole('button', { name: office.actions.more }));
    await waitFor(() => expect(f.query.mock.calls.at(-1)![0]).toMatchObject({ cursor: 'second-page' }));
    const other = status(); other.actor = { ...actor, user_id: 'another-user', name: 'Another user' }; f.setStatus(other);
    fireEvent.click(view.getByRole('button', { name: office.actions.refresh }));
    await view.findByText('Another user');
    await waitFor(() => expect(f.query.mock.calls.at(-1)![0]).toEqual({ kind: 'documents', query: '', filter: 'all', cursor: null }));
  });
  test('a cursor loop disables forward paging without hiding the current resources', async () => {
    const f = fixture();
    f.query.mockResolvedValue({ ...empty(), items: [{ ...item('doc', 'Existing document'), kind: 'docx', url: 'https://team.feishu.cn/docx/doc' }], has_more: true, next_cursor: 'repeated-page' });
    const view = await f.mount('/office?tab=documents'); await view.findByRole('button', { name: 'Existing document' });
    fireEvent.click(view.getByRole('button', { name: office.actions.more }));
    await view.findByText(office.resources.paginationError);
    expect((view.getByRole('button', { name: office.actions.more }) as HTMLButtonElement).disabled).toBe(true);
    expect(view.getByRole('button', { name: 'Existing document' })).toBeTruthy();
  });
  test('missing browse permission shows authorization and never an empty document claim', async () => {
    const s = status(); grant(s, 'documents.browse', false); const f = fixture(s), view = await f.mount('/office?tab=documents');
    expect(view.getByText(office.permissions.readMissing)).toBeTruthy(); expect(view.queryByText(office.resources.queryEmpty)).toBeNull();
    expect(f.query).not.toHaveBeenCalled(); expect(f.startAuth).not.toHaveBeenCalled();
  });
  test('opening a missing component never installs, authenticates or reads business data', async () => {
    const s = status(); s.runtime.phase = 'missing'; s.runtime.executable = null; s.phase = 'not_configured'; s.actor = null;
    const f = fixture(s), view = await f.mount();
    expect(f.prepare).not.toHaveBeenCalled(); expect(f.startAuth).not.toHaveBeenCalled(); expect(f.query).not.toHaveBeenCalled();
    fireEvent.click(view.getByRole('button', { name: office.actions.connect }));
    const modal = await view.findByRole('dialog', { name: office.actions.connect });
    expect(view.getAllByRole('dialog')).toHaveLength(1);
    expect(within(modal).getByText(office.setup.environment)).toBeTruthy();
    expect(within(modal).getByText('backend-machine · windows')).toBeTruthy();
    expect(within(modal).getByText(office.runtime.fullWizardHint)).toBeTruthy();
  });
  test('failed initial preparation can be explicitly retried without another installation mode', async () => {
    const s = status(); s.runtime.phase = 'missing'; s.runtime.executable = null; s.phase = 'not_configured'; s.actor = null;
    const f = fixture(s), failed = structuredClone(s); failed.runtime.phase = 'error'; failed.runtime.message = 'Download failed';
    f.prepare.mockImplementationOnce(async () => { f.setStatus(failed); throw new Error('Download failed'); }).mockImplementationOnce(async () => { const next = status(); next.actor = null; f.setStatus(next); return next.runtime; });
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.connect }));
    fireEvent.click(await view.findByRole('button', { name: office.actions.prepare }));
    const retry = await view.findByRole('button', { name: office.runtime.retryPrepare });
    await waitFor(() => expect((retry as HTMLButtonElement).disabled).toBe(false)); fireEvent.click(retry);
    await waitFor(() => expect(f.prepare).toHaveBeenCalledTimes(2));
    expect(f.prepare.mock.calls.every(call => call.length === 0)).toBe(true); expect(f.execute).not.toHaveBeenCalled();
  });
  for (const phase of ['broken','incompatible','error'] as const) test('an existing ' + phase + ' executable is never treated as a missing component', async () => {
    const s = status(); s.runtime.phase = phase; s.phase = 'error'; const f = fixture(s), view = await f.mount();
    fireEvent.click(view.getByRole('button', { name: office.actions.connect }));
    expect(view.queryByRole('button', { name: office.actions.prepare })).toBeNull(); expect(view.queryByRole('button', { name: office.runtime.retryPrepare })).toBeNull(); expect(f.prepare).not.toHaveBeenCalled();
    expect(view.getByText(office.runtime.brokenHint)).toBeTruthy();
  });
  test('business permissions may be skipped and do not imply empty data', async () => {
    const s = status(); s.permissions.forEach(permission => { permission.granted = false; });
    const f = fixture(s), view = await f.mount();
    fireEvent.click(view.getByRole('button', { name: office.connection.settings }));
    const modal = await view.findByRole('dialog'); fireEvent.click(within(modal).getByRole('button', { name: office.setup.skip }));
    expect(view.queryByRole('dialog')).toBeNull(); expect(view.getByText(office.permissions.readMissing)).toBeTruthy();
    expect(f.startAuth).not.toHaveBeenCalled(); expect(f.execute).not.toHaveBeenCalled();
    expect(f.query.mock.calls.every(([query]) => query.kind === 'recent')).toBe(true);
  });
  test('a disabled configured connection is explicitly enabled before its login flow', async () => {
    const s = status(); s.connection.enabled = false; s.phase = 'disabled'; const f = fixture(s);
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.connect }));
    await pressReady(view, office.auth.open);
    await waitFor(() => expect(f.startAuth).toHaveBeenCalledTimes(1));
    expect(f.updateConnection.mock.calls[0][0]).toEqual({ name: 'Work connection', enabled: true, profile: 'work' });
    expect(f.startAuth.mock.calls[0][0]).toMatchObject({ kind: 'login', profile: 'work' });
    expect(f.startAuth.mock.calls[0][0].scopes).toEqual(['offline_access']);
  });
  test('first disabled connection independently loads real profiles before selection and identity-only login', async () => {
    const s = status(); s.connection.enabled = false; s.connection.profile = null; s.actor = null; s.phase = 'disabled'; const f = fixture(s);
    f.profiles.mockResolvedValue([{ name: 'enterprise-ready', app_id: 'existing-app', brand: 'feishu', user_name: null, token_status: null }]);
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.connect })); const modal = within(view.getByRole('dialog'));
    fireEvent.click(modal.getByText(office.connection.existingApp)); fireEvent.click(modal.getByRole('checkbox', { name: office.connection.useExisting }));
    await modal.findByRole('option', { name: 'enterprise-ready · existing-app' });
    expect(f.profiles).toHaveBeenCalledTimes(1); expect(f.updateConnection).not.toHaveBeenCalled(); expect(f.startAuth).not.toHaveBeenCalled(); expect(f.prepare).not.toHaveBeenCalled();
    fireEvent.change(modal.getByLabelText(office.connection.profile), { target: { value: 'enterprise-ready' } }); await pressReady(view, office.auth.open);
    await waitFor(() => expect(f.startAuth).toHaveBeenCalledTimes(1));
    expect(f.updateConnection.mock.calls[0][0]).toEqual({ name: 'Work connection', enabled: true, profile: 'enterprise-ready' });
    expect(f.startAuth.mock.calls[0][0]).toMatchObject({ kind: 'login', profile: 'enterprise-ready', scopes: ['offline_access'] });
  });
  test('profile read failure retries only that read and new configuration uses the returned unique names', async () => {
    const s = status(); s.connection.profile = null; s.actor = null; s.phase = 'not_configured'; const f = fixture(s);
    f.profiles.mockRejectedValueOnce(new Error('Profile list unavailable')).mockResolvedValue([{ name: 'nomifun-office', app_id: 'existing-app', brand: 'feishu', user_name: null, token_status: null }]);
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.connect }));
    const error = await view.findByRole('alert'); expect(error.textContent).toContain('Profile list unavailable'); expect(f.updateConnection).not.toHaveBeenCalled(); expect(f.startAuth).not.toHaveBeenCalled();
    fireEvent.click(within(error).getByRole('button', { name: office.actions.check })); await pressReady(view, office.auth.open);
    await waitFor(() => expect(f.startAuth).toHaveBeenCalledTimes(1)); expect(f.profiles).toHaveBeenCalledTimes(2);
    expect(f.startAuth.mock.calls[0][0]).toMatchObject({ kind: 'configure_new', profile: 'nomifun-office-1' }); expect(f.updateConnection).not.toHaveBeenCalled();
  });
  test('new application configuration completes identity-only login before business reading can be skipped', async () => {
    const s = status(); s.actor = null; s.phase = 'not_configured'; s.connection.profile = null; s.permissions.forEach(permission => { permission.granted = false; }); const f = fixture(s);
    f.startAuth.mockImplementation(async request => {
      if (request.kind === 'configure_new') {
        s.connection.profile = request.profile; s.phase = 'needs_login'; f.setStatus(structuredClone(s)); return { ...flow('complete'), kind: 'configure_new' };
      }
      expect(request.scopes).toEqual(['offline_access']); s.actor = { ...actor }; s.phase = 'ready'; f.setStatus(structuredClone(s)); return flow('complete');
    });
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.connect }));
    await pressReady(view, office.auth.open); await waitFor(() => expect(f.startAuth).toHaveBeenCalledTimes(1));
    await waitFor(() => expect((view.getByRole('button', { name: office.auth.open }) as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(view.getByRole('button', { name: office.auth.open })); await waitFor(() => expect(f.startAuth).toHaveBeenCalledTimes(2));
    fireEvent.click(await view.findByRole('button', { name: office.actions.continue })); fireEvent.click(view.getByRole('button', { name: office.setup.skip }));
    expect(view.queryByRole('dialog')).toBeNull(); expect(view.getByText(office.permissions.readMissing)).toBeTruthy();
    expect(f.startAuth.mock.calls[0][0].kind).toBe('configure_new'); expect(f.startAuth.mock.calls[1][0].profile).toBe(f.startAuth.mock.calls[0][0].profile); expect(f.execute).not.toHaveBeenCalled();
    expect(f.query.mock.calls.every(([query]) => query.kind === 'recent')).toBe(true);
  });
  test('an unfinished identity login reopens at the connection step and observes its original flow', async () => {
    const s = status(); s.actor = null; s.phase = 'not_configured'; s.connection.profile = null; const f = fixture(s);
    f.startAuth.mockImplementation(async request => {
      if (request.kind === 'configure_new') { s.connection.profile = request.profile; s.phase = 'needs_login'; f.setStatus(structuredClone(s)); return { ...flow('complete'), kind: 'configure_new' }; }
      expect(request.scopes).toEqual(['offline_access']); return flow('waiting_user');
    });
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.connect }));
    await pressReady(view, office.auth.open); await waitFor(() => expect(f.startAuth).toHaveBeenCalledTimes(1));
    await waitFor(() => expect((view.getByRole('button', { name: office.auth.open }) as HTMLButtonElement).disabled).toBe(false)); fireEvent.click(view.getByRole('button', { name: office.auth.open }));
    await view.findByText(office.auth.phase.waiting_user); fireEvent.click(within(view.getByRole('dialog')).getAllByRole('button', { name: office.actions.close })[0]);
    fireEvent.click(view.getByRole('button', { name: office.actions.connect })); const modal = within(view.getByRole('dialog'));
    expect(modal.getByText(office.setup.connect).closest('li')?.getAttribute('aria-current')).toBe('step');
    expect(modal.queryByText(office.setup.permissionsTitle)).toBeNull(); expect(modal.getByText(office.auth.phase.waiting_user)).toBeTruthy();
    expect(f.startAuth).toHaveBeenCalledTimes(2); expect(f.execute).not.toHaveBeenCalled();
    await waitFor(() => expect(f.auth).toHaveBeenCalledWith(flow().id, expect.any(AbortSignal)), { timeout: 1600 });
  });
  test('switching a selected profile is not skipped because the old actor exists, and failed selection never logs in', async () => {
    const f = fixture(); f.profiles.mockResolvedValue([{ name: 'work', app_id: actor.app_id, brand: 'feishu', user_name: actor.name, token_status: 'valid' }, { name: 'other', app_id: 'other-app', brand: 'feishu', user_name: null, token_status: null }]);
    f.updateConnection.mockRejectedValue(new Error('Cannot select configuration'));
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.connection.settings }));
    let modal = view.getByRole('dialog'); fireEvent.click(within(modal).getByText(office.connection.advanced)); fireEvent.click(within(modal).getByRole('button', { name: office.actions.check }));
    fireEvent.click(within(modal).getByRole('button', { name: office.actions.continue }));
    modal = view.getByRole('dialog'); fireEvent.click(within(modal).getByText(office.connection.existingApp));
    await within(modal).findByRole('option', { name: 'other · other-app' });
    fireEvent.change(within(modal).getByLabelText(office.connection.profile), { target: { value: 'other' } });
    fireEvent.click(within(modal).getByRole('button', { name: office.auth.open }));
    await view.findByText('Cannot select configuration');
    expect(f.updateConnection.mock.calls[0][0].profile).toBe('other'); expect(f.startAuth).not.toHaveBeenCalled();
  });
  test('title-only document creation is a single explicit native save', async () => {
    const f = fixture(), view = await f.mount();
    fireEvent.change(view.getByLabelText(office.resources.search), { target: { value: 'Earlier search' } });
    fireEvent.submit(view.getByLabelText(office.resources.search).closest('form')!);
    fireEvent.click(view.getByRole('button', { name: office.actions.new }));
    fireEvent.change(view.getByLabelText(office.documents.title), { target: { value: 'An empty document' } });
    fireEvent.click(view.getByRole('button', { name: office.actions.create }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
    const sent = f.execute.mock.calls[0][0];
    expect(sent.items).toHaveLength(1); expect(sent.items[0]).toMatchObject({ kind: 'document_create', title: 'An empty document' }); expect(sent.expected_actor).toEqual(actor);
    await waitFor(() => expect(view.queryByRole('dialog')).toBeNull());
    expect(f.success).toHaveBeenCalledTimes(1);
    expect(f.success).toHaveBeenCalledWith({ content: office.saved, duration: 3000 });
    expect(view.queryByText(office.saved)).toBeNull();
    expect((view.getByLabelText(office.resources.search) as HTMLInputElement).value).toBe('');
    expect(f.external).not.toHaveBeenCalled();
  });
  for (const phase of ['loading', 'queued', 'running'] as const) test('ordinary save ' + phase + ' shows neutral progress without network recovery controls', async () => {
    const f = fixture(); let finish: () => void = () => {};
    f.execute.mockImplementation(async request => phase === 'loading' ? await new Promise<OfficeOperation>(resolve => { finish = () => resolve(receipt(request)); }) : receipt(request, phase));
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.new }));
    fireEvent.change(view.getByLabelText(office.documents.title), { target: { value: 'A normal save' } }); fireEvent.click(view.getByRole('button', { name: office.actions.create }));
    const modal = within(view.getByRole('dialog')); await modal.findByText(phase === 'loading' ? office.operation.saving : office.operation.processing);
    expect(modal.queryByText(office.operation.networkHint)).toBeNull(); expect(modal.queryByRole('button', { name: office.operation.checkReceipt })).toBeNull(); expect(modal.queryByRole('button', { name: office.actions.details })).toBeNull();
    expect(f.success).not.toHaveBeenCalled();
    if (phase === 'loading') { await act(async () => { finish(); }); await waitFor(() => expect(view.queryByRole('dialog')).toBeNull()); }
  });
  test('a single task uses verified identity and optional due date without batch controls', async () => {
    const f = fixture(), view = await f.mount('/office?tab=agenda');
    fireEvent.click(view.getByRole('button', { name: office.tasks.section }));
    fireEvent.change(view.getByLabelText(office.tasks.title), { target: { value: 'Manual task' } });
    fireEvent.click(view.getByRole('button', { name: office.actions.create }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
    expect(f.execute.mock.calls[0][0].items).toEqual([expect.objectContaining({ kind: 'task_create', title: 'Manual task', assignee_id: actor.user_id, due: null, tasklist_id: null })]);
    expect(view.queryByText(/Add task|AI|Preview|Return to draft/)).toBeNull();
  });
  test('a new native form has a new logical ID while an explicitly unexecuted form retains its ID', async () => {
    const f = fixture(); f.execute.mockImplementation(async request => receipt(request, 'not_executed'));
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.new }));
    fireEvent.change(view.getByLabelText(office.documents.title), { target: { value: 'First logical document' } });
    fireEvent.click(view.getByRole('button', { name: office.actions.create })); await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
    fireEvent.change(view.getByLabelText(office.documents.title), { target: { value: 'Edited same document' } }); fireEvent.click(view.getByRole('button', { name: office.actions.create })); await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(2));
    expect(f.execute.mock.calls[0][0].items[0].client_item_id).toBe(f.execute.mock.calls[1][0].items[0].client_item_id);
    fireEvent.click(view.getByRole('button', { name: office.actions.cancel })); fireEvent.click(view.getByRole('button', { name: office.actions.new }));
    fireEvent.change(view.getByLabelText(office.documents.title), { target: { value: 'A separate document' } }); fireEvent.click(view.getByRole('button', { name: office.actions.create })); await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(3));
    expect(f.execute.mock.calls[2][0].items[0].client_item_id).not.toBe(f.execute.mock.calls[1][0].items[0].client_item_id);
  });
  test('personal event form sends an explicit local timezone without adding attendees', async () => {
    const f = fixture(), view = await f.mount('/office?tab=agenda'); fireEvent.click(view.getByRole('button', { name: office.calendar.section }));
    const modal = within(view.getByRole('dialog')); fireEvent.change(modal.getByLabelText(office.calendar.title), { target: { value: 'Focus time' } }); fireEvent.change(modal.getByLabelText(office.calendar.date), { target: { value: '2026-10-27' } }); fireEvent.change(modal.getByLabelText(office.calendar.time), { target: { value: '14:00' } }); fireEvent.click(modal.getByRole('button', { name: office.actions.create }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1)); const mutation = f.execute.mock.calls[0][0].items[0];
    expect(mutation.kind).toBe('calendar_create'); if (mutation.kind !== 'calendar_create') throw new Error('Wrong mutation');
    expect(mutation.timezone).toBe(Intl.DateTimeFormat().resolvedOptions().timeZone); expect(Date.parse(mutation.end) - Date.parse(mutation.start)).toBe(30 * 60_000); expect('attendees' in mutation).toBe(false);
  });
  test('write permission returns to identical input and never automatically creates', async () => {
    const s = status(); grant(s, 'documents.create', false);
    const f = fixture(s); f.startAuth.mockImplementation(async () => { const next = structuredClone(s); grant(next, 'documents.create', true); f.setStatus(next); return flow('complete'); });
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.new }));
    fireEvent.change(view.getByLabelText(office.documents.title), { target: { value: 'Keep my input' } });
    fireEvent.click(view.getByRole('button', { name: office.actions.create }));
    await view.findByRole('dialog', { name: office.auth.title }); expect(f.execute).not.toHaveBeenCalled();
    fireEvent.click(view.getByRole('button', { name: office.auth.return }));
    expect((view.getByLabelText(office.documents.title) as HTMLInputElement).value).toBe('Keep my input');
    expect(f.execute).not.toHaveBeenCalled(); fireEvent.click(view.getByRole('button', { name: office.actions.create }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
  });
  test('an authorization identity change blocks the old form instead of rebinding it', async () => {
    const s = status(); grant(s, 'tasks.write', false); const f = fixture(s);
    f.startAuth.mockImplementation(async () => { const next = structuredClone(s); next.actor = { ...actor, user_id: 'other-user' }; grant(next, 'tasks.write', true); f.setStatus(next); return flow('complete'); });
    const view = await f.mount('/office?tab=agenda'); fireEvent.click(view.getByRole('button', { name: office.tasks.section }));
    fireEvent.change(view.getByLabelText(office.tasks.title), { target: { value: 'Bound to old user' } }); fireEvent.click(view.getByRole('button', { name: office.actions.create }));
    await view.findByText(office.auth.phase.complete); fireEvent.click(view.getByRole('button', { name: office.auth.return }));
    await view.findByText(office.operation.identityChangedInput);
    expect((view.getByRole('button', { name: office.actions.create }) as HTMLButtonElement).disabled).toBe(true); expect(f.execute).not.toHaveBeenCalled();
  });
  test('closing authorization keeps its original cancelling flow observable', async () => {
    const s = status(); grant(s, 'documents.create', false); const f = fixture(s); f.auth.mockResolvedValue(flow('cancelling'));
    const view = await f.mount(); fireEvent.click(view.getByRole('button', { name: office.actions.new })); fireEvent.change(view.getByLabelText(office.documents.title), { target: { value: 'Temporary input' } }); fireEvent.click(view.getByRole('button', { name: office.actions.create }));
    await view.findByText(office.auth.phase.waiting_user); fireEvent.click(view.getByRole('button', { name: office.auth.cancel })); await view.findByText(office.auth.phase.cancelling);
    fireEvent.click(view.getByRole('button', { name: office.auth.return })); expect((view.getByLabelText(office.documents.title) as HTMLInputElement).value).toBe('Temporary input');
    fireEvent.click(view.getByRole('button', { name: office.actions.create })); await view.findByRole('dialog', { name: office.auth.title });
    expect(f.startAuth).toHaveBeenCalledTimes(1); expect(f.execute).not.toHaveBeenCalled(); expect(view.getAllByRole('dialog')).toHaveLength(1);
    await waitFor(() => expect(f.auth).toHaveBeenCalled(), { timeout: 1600 });
  });
  test('history shows safe summaries and original pending receipt with no retry editor', async () => {
    const f = fixture(); const op = receipt({ request_id: 'request', expected_actor: actor, profile: 'work', items: [{ kind: 'document_create', client_item_id: 'logical', title: 'Original title' }] }, 'unknown');
    f.history.mockResolvedValue({ items: [op], next_cursor: null }); f.open.mockResolvedValue(op); f.reconcile.mockResolvedValue(op);
    const view = await f.mount('/office?tab=activity'); fireEvent.click(await view.findByRole('button', { name: office.actions.details }));
    await view.findByText('Original title'); expect(view.queryByRole('textbox')).toBeNull();
    const verify = view.getByRole('button', { name: office.actions.reconcile }); expect((verify as HTMLButtonElement).disabled).toBe(false); fireEvent.click(verify); await waitFor(() => expect(f.reconcile).toHaveBeenCalledTimes(1)); expect(f.execute).not.toHaveBeenCalled();
  });
});
