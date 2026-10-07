import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IRequirement } from '@/common/adapter/ipcBridge';
import { parseRequirementId } from '@/common/types/ids';
import WorkspacePage from './index';
import RequirementListView from './RequirementListView';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(fn => fn()); });
function deferred<T>() {
  let resolve!: (value: T) => void; let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const item = (n: number): IRequirement => ({ requirement_id: parseRequirementId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0')),
  display_no: n, title: 'requirement-' + n, content: '', tag: 'fixture', order_key: '', status: 'pending', attempt_count: 0, created_by: 'user', created_at: 1, updated_at: 1 });
function fixture() {
  const error = mock(() => () => {});
  const messages = spyOn(Message, 'useMessage').mockReturnValue([{ error, success: () => () => {} }, <></>]);
  const list = spyOn(ipcBridge.requirements.list, 'invoke').mockResolvedValue({ items: [item(1), item(2)], total: 2, has_more: false });
  const tags = spyOn(ipcBridge.requirements.tags, 'invoke').mockResolvedValue([]);
  restore.push(() => messages.mockRestore(), () => list.mockRestore(), () => tags.mockRestore());
  for (const source of [ipcBridge.requirements.onCreated, ipcBridge.requirements.onUpdated, ipcBridge.requirements.onStatusChanged,
    ipcBridge.requirements.onDeleted, ipcBridge.requirements.onTagPaused, ipcBridge.conversation.reconnected]) {
    const spy = spyOn(source, 'on').mockImplementation(() => () => {}); restore.push(() => spy.mockRestore());
  }
  return { error, list };
}
const mount = () => render(<I18nextProvider i18n={i18n}><MemoryRouter><WorkspacePage /></MemoryRouter></I18nextProvider>);
const rows = (v: ReturnType<typeof render>) => v.container.querySelectorAll<HTMLElement>('.requirements-list-row');
async function startMutation(v: ReturnType<typeof render>, kind: 'status' | 'delete' | 'batch') {
  if (kind === 'status') {
    fireEvent.click(within(rows(v)[0]!).getByText('requirements.status.pending'));
    fireEvent.click(await v.findByRole('menuitem', { name: 'requirements.status.done' }));
  } else {
    if (kind === 'batch') {
      fireEvent.click(within(rows(v)[0]!).getByRole('checkbox'));
      fireEvent.click(v.getByText('requirements.actions.delete'));
    } else fireEvent.click(within(rows(v)[0]!).getByLabelText('requirements.actions.delete'));
    await act(async () => {});
    fireEvent.click(v.baseElement.querySelector('.arco-popconfirm-btn .arco-btn-primary')!);
  }
  await act(async () => {});
}

test('an emptied last page retains pagination to reach remaining requirements', () => {
  const change = mock();
  const v = render(<I18nextProvider i18n={i18n}><RequirementListView items={[]} total={20} page={2} pageSize={20}
    onPageChange={change} selectedIds={new Set()} onToggleSelect={() => {}} onOpenDetail={() => {}} onStatusChange={() => {}}
    onEdit={() => {}} onDelete={() => {}} onCreate={() => {}} /></I18nextProvider>);
  expect(v.container.querySelector('.requirements-pagination')).not.toBeNull();
  fireEvent.click(v.getByText('1'));
  expect(change).toHaveBeenCalledWith(1, 20);
});

test('batch completion preserves selections added while the request was pending', async () => {
  fixture(); const pending = deferred<{ deleted: number }>();
  const remove = spyOn(ipcBridge.requirements.batchDelete, 'invoke').mockImplementation(() => pending.promise); restore.push(() => remove.mockRestore());
  const v = mount(); await act(async () => {});
  await startMutation(v, 'batch'); expect(remove).toHaveBeenCalledWith({ requirement_ids: [item(1).requirement_id] });
  fireEvent.click(within(rows(v)[1]!).getByRole('checkbox'));
  await act(async () => { pending.resolve({ deleted: 1 }); });
  expect((within(rows(v)[0]!).getByRole('checkbox') as HTMLInputElement).checked).toBe(false);
  expect((within(rows(v)[1]!).getByRole('checkbox') as HTMLInputElement).checked).toBe(true);
});

test.each(['status', 'delete', 'batch'] as const)('%s failures are visible and late failures stay silent after unmount', async kind => {
  const f = fixture(); const failure = new Error('write failed'); const pending = deferred<never>(); let call = 0;
  const fail = () => ++call === 1 ? Promise.reject(failure) : pending.promise;
  const update = spyOn(ipcBridge.requirements.update, 'invoke').mockImplementation(fail);
  const remove = spyOn(ipcBridge.requirements.remove, 'invoke').mockImplementation(fail);
  const batch = spyOn(ipcBridge.requirements.batchDelete, 'invoke').mockImplementation(fail);
  restore.push(() => update.mockRestore(), () => remove.mockRestore(), () => batch.mockRestore());
  const v = mount(); await act(async () => {}); await startMutation(v, kind);
  expect(f.error).toHaveBeenCalledWith(String(failure));
  // Batch selection remains after failure; reopen confirmation without toggling it off.
  if (kind === 'batch') fireEvent.click(within(rows(v)[0]!).getByRole('checkbox'));
  await startMutation(v, kind); expect(call).toBe(2);
  v.unmount(); await act(async () => { pending.reject(new Error('late failure')); });
  expect(f.error).toHaveBeenCalledTimes(1);
});
