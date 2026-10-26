import { afterEach, beforeAll, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { createMemoryRouter, RouterProvider } from 'react-router-dom';
import { officeHub } from '@/common/adapter/officeHubBridge';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import type { AuthorizationFlow, ExecuteRequest, OfficeDocument, OfficeItem, OfficeOperation, OfficeQueryResult, OfficeStatus } from '@/common/types/officeHub';
import office from '@/renderer/services/i18n/locales/en-US/office.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import OfficeHubPage from './index';

const locale = createInstance();
const restores: Array<() => void> = [];
const track = <T extends { mockRestore(): void }>(spy: T) => { restores.push(() => spy.mockRestore()); return spy; };
const actor = { app_id: 'real-app', user_id: 'real-user', name: 'Verified user' };
const empty = (): OfficeQueryResult => ({ items: [], document: null, fields: [], has_more: false, next_cursor: null, warnings: [] });
function status(): OfficeStatus {
  return { provider: 'feishu', connection: { id: 'connection', name: 'Work connection', enabled: true, profile: 'work', executable: null, created_at: '2026-10-26T00:00:00Z', updated_at: '2026-10-26T00:00:00Z' }, runtime: { phase: 'ready', executable: 'C:/tools/lark-cli.exe', version: '1.0.98', source: 'external', message: null, official_install_command: 'npx @larksuite/cli@latest install' }, phase: 'ready', actor: { ...actor }, host_name: 'backend-machine', host_os: 'windows', message: null, permissions: ['documents.search','documents.read','documents.create','documents.write','people.read','tasks.read','tasks.write','tasklists.read','calendar.read','calendar.write','base.read','base.write'].map(capability => ({ capability, granted: true, required_scopes: [capability + '-scope'] })) };
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
function document(): OfficeDocument {
  return { id: 'actual-document', title: 'A real document', content: 'Original paragraph', url: 'https://team.feishu.cn/docx/actual-document', pending_operation_id: null, pending_status: null, edit_baseline: { revision: '28', blocks: [{ id: 'real-block-28', kind: 'paragraph', content: 'Original paragraph', editable: true, readonly_reason: null }, { id: 'heading-block', kind: 'heading', content: 'Readonly heading', editable: false, readonly_reason: 'structure' }], can_insert_first: false } };
}
beforeAll(async () => { await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: { office, common } } } }); });
afterEach(() => { cleanup(); restores.splice(0).reverse().forEach(restore => restore()); });
function fixture(initial = status()) {
  let current = initial;
  const success = track(spyOn(Message, 'success').mockReturnValue(() => {}));
  const getStatus = track(spyOn(officeHub, 'status').mockImplementation(async () => structuredClone(current)));
  const profiles = track(spyOn(officeHub, 'profiles').mockResolvedValue([{ name: 'work', app_id: actor.app_id, brand: 'feishu', user_name: actor.name, token_status: 'valid' }]));
  const query = track(spyOn(officeHub, 'query').mockImplementation(async () => empty()));
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
  return { mount, success, getStatus, profiles, query, history, prepare, execute, request, reconcile, open, auth, startAuth, cancelAuth, updateConnection, setStatus: (next: OfficeStatus) => { current = next; } };
}
async function pressReady(view: ReturnType<typeof render>, name: string) {
  const button = await view.findByRole('button', { name });
  await waitFor(() => expect((button as HTMLButtonElement).disabled).toBe(false));
  await act(async () => { fireEvent.click(button); });
}
const grant = (source: OfficeStatus, capability: string, value: boolean) => { source.permissions.find(permission => permission.capability === capability)!.granted = value; };

describe('compact Office Hub with isolated real API boundaries', () => {
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
    fireEvent.click(view.getByRole('button', { name: office.actions.new }));
    fireEvent.change(view.getByLabelText(office.documents.title), { target: { value: 'An empty document' } });
    fireEvent.click(view.getByRole('button', { name: office.actions.create }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
    const sent = f.execute.mock.calls[0][0];
    expect(sent.items).toHaveLength(1); expect(sent.items[0]).toMatchObject({ kind: 'document_create', title: 'An empty document', content: '' }); expect(sent.expected_actor).toEqual(actor);
    await waitFor(() => expect(view.queryByRole('dialog')).toBeNull());
    expect(f.success).toHaveBeenCalledTimes(1);
    expect(f.success).toHaveBeenCalledWith({ content: office.saved, duration: 3000 });
    expect(view.queryByText(office.saved)).toBeNull();
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
  test('document reading renders one title while preserving body text and Markdown structure', async () => {
    const f = fixture();
    const title = 'nomifun-test-3';
    const content = [title, '正文包含 **重点** 和 [参考链接](https://example.com/reference)。', '\\[内容\\]', String.raw`C:\\Users\\test\[draft\]\\file\(1\).md`, '## 后续章节', '- 第一项\n- 第二项', '| 列名 | 内容 |\n| --- | --- |\n| 状态 | 正常 |', '# 正文一级标题'].join('\n\n');
    // The read API separates the native export title from its Markdown body.
    f.query.mockImplementation(async query => query.kind === 'document' ? { ...empty(), document: { ...document(), title, content, edit_baseline: null } } : empty());
    const view = await f.mount('/office?tab=documents&document=actual-document');
    const article = await view.findByRole('article');
    expect(within(article).getAllByRole('heading', { name: title })).toHaveLength(1);
    const body = article.querySelector('.markdown-shadow')!.shadowRoot!;
    expect(body.querySelector('p')?.textContent).toBe(title);
    expect(body.querySelector('strong')?.textContent).toBe('重点');
    expect(body.querySelector('a')?.getAttribute('href')).toBe('https://example.com/reference');
    expect(body.querySelector('h2')?.textContent).toBe('后续章节');
    expect(body.querySelectorAll('li')).toHaveLength(2);
    expect(body.querySelector('td')?.textContent).toBe('状态');
    expect(body.querySelector('h1')?.textContent).toBe('正文一级标题');
    const paragraphs = Array.from(body.querySelectorAll('p')).map(paragraph => paragraph.textContent);
    expect(paragraphs).toContain('[内容]');
    expect(paragraphs).toContain(String.raw`C:\Users\test[draft]\file(1).md`);
    expect(body.querySelector('.katex')).toBeNull();
    expect(f.execute).not.toHaveBeenCalled();
  });
  test('paragraph save uses the real block ID and selection revision after authorization refresh', async () => {
    const s = status(); grant(s, 'documents.write', false); const f = fixture(s); let revision = '28';
    f.query.mockImplementation(async query => { if (query.kind !== 'document') return empty(); const doc = document(); doc.edit_baseline!.revision = revision; return { ...empty(), document: doc }; });
    f.startAuth.mockImplementation(async () => { const next = structuredClone(s); grant(next, 'documents.write', true); revision = '29'; f.setStatus(next); return flow('complete'); });
    const view = await f.mount('/office?tab=documents&document=https%3A%2F%2Fteam.feishu.cn%2Fdocx%2Factual-document');
    fireEvent.click(await view.findByRole('button', { name: office.actions.edit }));
    fireEvent.click(await view.findByRole('button', { name: 'Edit paragraph: Original paragraph' }));
    fireEvent.change(view.getByLabelText(office.documents.paragraph), { target: { value: 'Changed one paragraph' } }); fireEvent.click(view.getByRole('button', { name: office.actions.save }));
    await view.findByText(office.auth.phase.complete); expect(f.execute).not.toHaveBeenCalled(); fireEvent.click(view.getByRole('button', { name: office.auth.return }));
    expect((view.getByLabelText(office.documents.paragraph) as HTMLTextAreaElement).value).toBe('Changed one paragraph');
    fireEvent.click(view.getByRole('button', { name: office.actions.save })); await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
    expect(f.execute.mock.calls[0][0].items[0]).toMatchObject({ kind: 'document_save', document_id: 'actual-document', revision: '28', block_id: 'real-block-28', content: 'Changed one paragraph' });
    expect(f.query.mock.calls.some(([query]) => query.kind === 'document' && query.editing === true)).toBe(true);
  });
  test('paragraph save waits neutrally in its file bar without showing a network failure', async () => {
    const f = fixture(); let finish: () => void = () => {};
    f.query.mockImplementation(async query => query.kind === 'document' ? { ...empty(), document: document() } : empty());
    f.execute.mockImplementation(async request => await new Promise<OfficeOperation>(resolve => { finish = () => resolve(receipt(request)); }));
    const view = await f.mount('/office?tab=documents&document=actual-document'); fireEvent.click(await view.findByRole('button', { name: office.actions.edit })); fireEvent.click(await view.findByRole('button', { name: 'Edit paragraph: Original paragraph' }));
    fireEvent.change(view.getByLabelText(office.documents.paragraph), { target: { value: 'One changed paragraph' } }); fireEvent.click(view.getByRole('button', { name: office.actions.save }));
    await waitFor(() => expect(view.getAllByText(office.operation.saving).length).toBe(2));
    expect(view.queryByText(office.operation.networkHint)).toBeNull(); expect(view.queryByRole('button', { name: office.operation.checkReceipt })).toBeNull();
    await act(async () => { finish(); }); await waitFor(() => expect(f.success).toHaveBeenCalledTimes(1));
    expect(f.success).toHaveBeenCalledWith({ content: office.saved, duration: 3000 });
    expect(view.queryByText(office.saved)).toBeNull();
  });
  test('only a verified empty baseline offers first-paragraph insertion', async () => {
    const f = fixture(); f.query.mockImplementation(async query => query.kind === 'document' ? { ...empty(), document: { ...document(), content: '', edit_baseline: { revision: '1', blocks: [], can_insert_first: true } } } : empty());
    const view = await f.mount('/office?tab=documents&document=empty-doc'); fireEvent.click(await view.findByRole('button', { name: office.actions.edit }));
    fireEvent.click(await view.findByRole('button', { name: office.documents.firstParagraph })); fireEvent.change(view.getByLabelText(office.documents.paragraph), { target: { value: 'First paragraph' } }); fireEvent.click(view.getByRole('button', { name: office.actions.save }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1)); expect(f.execute.mock.calls[0][0].items[0]).toMatchObject({ kind: 'document_save', block_id: null, revision: '1' });
  });
  test('an entirely readonly baseline explains each block instead of claiming editing is active', async () => {
    const f = fixture(); const doc = document();
    doc.edit_baseline!.blocks = [
      { id: 'heading', kind: 'heading', content: 'Native heading', editable: false, readonly_reason: 'structure' },
      { id: 'formatted', kind: 'paragraph', content: 'Formatted text', editable: false, readonly_reason: 'rich_text' },
      { id: 'metadata', kind: 'paragraph', content: 'Protected text', editable: false, readonly_reason: 'metadata' },
    ];
    f.query.mockImplementation(async query => query.kind === 'document' ? { ...empty(), document: doc } : empty());
    const view = await f.mount('/office?tab=documents&document=actual-document');
    fireEvent.click(await view.findByRole('button', { name: office.actions.edit }));
    await view.findByText(office.documents.readonly);
    for (const reason of Object.values(office.documents.readonlyReason)) expect(view.getByText(reason)).toBeTruthy();
    expect(view.queryByText(office.documents.editing)).toBeNull();
    expect(view.queryByRole('textbox')).toBeNull();
    expect((view.getByRole('button', { name: office.actions.save }) as HTMLButtonElement).disabled).toBe(true);
    expect(f.execute).not.toHaveBeenCalled();
  });
  test('native paragraph text is clicked, focused and saved literally through repeated edits', async () => {
    const f = fixture(); const doc = document();
    const original = '**literal** [brackets] <tag> & C:\\Users\\中文\nsecond line';
    doc.edit_baseline!.blocks[0].content = original;
    f.query.mockImplementation(async query => query.kind === 'document' ? { ...empty(), document: structuredClone(doc) } : empty());
    f.execute.mockImplementation(async request => {
      const mutation = request.items[0];
      if (mutation.kind !== 'document_save') throw new Error('Expected a native paragraph save');
      doc.edit_baseline!.blocks[0].content = mutation.content;
      doc.edit_baseline!.revision = String(Number(doc.edit_baseline!.revision) + 1);
      return receipt(request);
    });
    const view = await f.mount('/office?tab=documents&document=actual-document');
    fireEvent.click(await view.findByRole('button', { name: office.actions.edit }));
    await view.findByText(office.documents.selectParagraph);
    const body = await view.findByRole('button', { name: /^Edit paragraph:/ });
    expect(body.textContent).toBe(original);
    expect(body.querySelector('strong, a, .markdown-shadow')).toBeNull();
    fireEvent.click(body.querySelector('span')!);
    const editor = view.getByLabelText(office.documents.paragraph) as HTMLTextAreaElement;
    expect(globalThis.document.activeElement).toBe(editor);
    expect(editor.value).toBe(original);
    const updated = original + '\n# literal heading * text';
    fireEvent.change(editor, { target: { value: updated } });
    fireEvent.click(view.getByRole('button', { name: office.actions.save }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
    expect(f.execute.mock.calls[0][0].items[0]).toMatchObject({ content: updated, revision: '28', block_id: 'real-block-28' });
    fireEvent.click(await view.findByRole('button', { name: office.actions.edit }));
    fireEvent.click(await view.findByRole('button', { name: /^Edit paragraph:/ }));
    expect((view.getByLabelText(office.documents.paragraph) as HTMLTextAreaElement).value).toBe(updated);
    fireEvent.change(view.getByLabelText(office.documents.paragraph), { target: { value: updated + ' again' } });
    fireEvent.click(view.getByRole('button', { name: office.actions.save }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(2));
    expect(f.execute.mock.calls[1][0].items[0]).toMatchObject({ content: updated + ' again', revision: '29' });
  });
  test('an existing empty native paragraph offers a focused editor for its actual block', async () => {
    const f = fixture(); const doc = document(); doc.edit_baseline!.blocks[0].content = '';
    f.query.mockImplementation(async query => query.kind === 'document' ? { ...empty(), document: doc } : empty());
    const view = await f.mount('/office?tab=documents&document=actual-document');
    fireEvent.click(await view.findByRole('button', { name: office.actions.edit }));
    fireEvent.click(await view.findByText(office.documents.emptyBlock));
    const editor = view.getByLabelText(office.documents.paragraph);
    expect(globalThis.document.activeElement).toBe(editor);
    fireEvent.change(editor, { target: { value: 'Fill the native empty paragraph' } });
    fireEvent.click(view.getByRole('button', { name: office.actions.save }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1));
    expect(f.execute.mock.calls[0][0].items[0]).toMatchObject({ block_id: 'real-block-28', content: 'Fill the native empty paragraph' });
  });
  test('a failed editing baseline can be explicitly reread without leaving the document', async () => {
    const f = fixture(); let fail = true;
    f.query.mockImplementation(async query => {
      if (query.kind !== 'document') return empty();
      if (query.editing && fail) { fail = false; throw new Error('Native baseline unavailable'); }
      return { ...empty(), document: document() };
    });
    const view = await f.mount('/office?tab=documents&document=actual-document');
    fireEvent.click(await view.findByRole('button', { name: office.actions.edit }));
    await view.findByText('Native baseline unavailable');
    fireEvent.click(view.getByRole('button', { name: office.documents.retryRead }));
    await view.findByRole('button', { name: 'Edit paragraph: Original paragraph' });
    expect(f.execute).not.toHaveBeenCalled();
  });
  test('a revision conflict retains unsaved text but requires choosing the current paragraph after rereading', async () => {
    const f = fixture(); const doc = document();
    f.query.mockImplementation(async query => query.kind === 'document' ? { ...empty(), document: structuredClone(doc) } : empty());
    f.execute.mockImplementation(async () => {
      doc.edit_baseline!.revision = '29'; doc.edit_baseline!.blocks[0].content = 'Concurrent native edit';
      throw new BackendHttpError({ method: 'POST', path: '/api/office-hub/execute', status: 409, body: { error: 'The document changed', code: 'OFFICE_CONFLICT', details: { not_executed: true } } });
    });
    const view = await f.mount('/office?tab=documents&document=actual-document');
    fireEvent.click(await view.findByRole('button', { name: office.actions.edit }));
    fireEvent.click(await view.findByText('Original paragraph'));
    fireEvent.change(view.getByLabelText(office.documents.paragraph), { target: { value: 'Keep this unsaved draft' } });
    fireEvent.click(view.getByRole('button', { name: office.actions.save }));
    await view.findByText('The document changed');
    expect((view.getByRole('button', { name: office.actions.save }) as HTMLButtonElement).disabled).toBe(true);
    expect((view.getByLabelText(office.documents.paragraph) as HTMLTextAreaElement).value).toBe('Keep this unsaved draft');
    fireEvent.click(view.getByRole('button', { name: office.documents.reread }));
    await view.findByText('Concurrent native edit');
    expect((view.getByLabelText(office.documents.retainedInput) as HTMLTextAreaElement).value).toBe('Keep this unsaved draft');
    expect(view.queryByLabelText(office.documents.paragraph)).toBeNull();
    expect((view.getByRole('button', { name: office.actions.save }) as HTMLButtonElement).disabled).toBe(true);
    expect(f.execute).toHaveBeenCalledTimes(1);
    fireEvent.click(view.getByText('Concurrent native edit'));
    expect((view.getByLabelText(office.documents.paragraph) as HTMLTextAreaElement).value).toBe('Concurrent native edit');
    expect(f.execute).toHaveBeenCalledTimes(1);
  });
  test('Base row pending facts disable selection and typed selection saves actual field IDs', async () => {
    const f = fixture(); const one = { ...item('row-one','First customer'), kind: 'base_record', fields: { fldActual: ['New'] } }; const pending = { ...item('row-pending','Pending customer'), kind: 'base_record', fields: { fldActual: ['New'] }, pending_operation_id: 'original', pending_status: 'accepted_pending_confirmation' as const };
    f.query.mockImplementation(async query => query.kind === 'base_records' ? { ...empty(), fields: [{ id: 'fldActual', name: 'Status', field_type: 'select', writable: true, options: [{ id: 'opaque-option-id', name: 'Contacted' }, { name: 'New' }] }], items: [one, pending] } : empty());
    const view = await f.mount('/office?tab=base'); fireEvent.change(view.getByLabelText(office.base.link), { target: { value: 'https://team.feishu.cn/base/baseA?table=tblA' } }); fireEvent.click(view.getByRole('button', { name: office.base.open }));
    const pendingBox = await view.findByRole('checkbox', { name: 'Select Pending customer' }); expect((pendingBox as HTMLInputElement).disabled).toBe(true);
    fireEvent.click(view.getByRole('checkbox', { name: office.base.selectAvailable })); fireEvent.click(view.getByRole('button', { name: office.base.batch }));
    fireEvent.change(view.getByLabelText(office.base.value), { target: { value: 'Contacted' } }); fireEvent.click(view.getByRole('button', { name: office.actions.save }));
    await waitFor(() => expect(f.execute).toHaveBeenCalledTimes(1)); expect(f.execute.mock.calls[0][0].items).toEqual([expect.objectContaining({ record_id: 'row-one', fields: { fldActual: ['Contacted'] }, before: { fldActual: ['New'] } })]);
    expect(f.query.mock.calls.find(([query]) => query.kind === 'base_records')?.[0]).toMatchObject({ base_token: 'baseA', table_id: 'tblA', filter: null });
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
    const f = fixture(); const op = receipt({ request_id: 'request', expected_actor: actor, profile: 'work', items: [{ kind: 'document_create', client_item_id: 'logical', title: 'Original title', content: '' }] }, 'unknown');
    f.history.mockResolvedValue({ items: [op], next_cursor: null }); f.open.mockResolvedValue(op); f.reconcile.mockResolvedValue(op);
    const view = await f.mount('/office?tab=activity'); fireEvent.click(await view.findByRole('button', { name: office.actions.details }));
    await view.findByText('Original title'); expect(view.queryByRole('textbox')).toBeNull();
    const verify = view.getByRole('button', { name: office.actions.reconcile }); expect((verify as HTMLButtonElement).disabled).toBe(false); fireEvent.click(verify); await waitFor(() => expect(f.reconcile).toHaveBeenCalledTimes(1)); expect(f.execute).not.toHaveBeenCalled();
  });
});
