import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { Form, Message, type FormInstance } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { IRequirement } from '@/common/adapter/ipcBridge';
import { parseRequirementId } from '@/common/types/ids';
import RequirementDrawer from './index';
import RequirementForm from './RequirementForm';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach((dispose) => dispose()); });
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const item = (n: number): IRequirement => ({
  requirement_id: parseRequirementId('019b0000-0000-7000-8000-' + String(n).padStart(12, '0')),
  display_no: n, title: 'requirement-' + n, content: '', tag: 'fixture', order_key: '', status: 'pending',
  attempt_count: 0, created_by: 'user', created_at: 1, updated_at: 1,
});
function fixture() {
  let form: unknown;
  const original = Form.useForm;
  const useForm = spyOn(Form, 'useForm').mockImplementation((existing) => {
    const result = original(existing); form = result[0]; return result;
  });
  const tags = spyOn(ipcBridge.requirements.tags, 'invoke').mockResolvedValue([]);
  const notices = spyOn(Message, 'useMessage').mockReturnValue([{ success: () => () => {}, error: () => () => {} }, <></>]);
  restore.push(() => useForm.mockRestore(), () => tags.mockRestore(), () => notices.mockRestore());
  for (const source of [ipcBridge.requirements.onCreated, ipcBridge.requirements.onUpdated,
    ipcBridge.requirements.onStatusChanged, ipcBridge.requirements.onDeleted,
    ipcBridge.requirements.onTagPaused, ipcBridge.conversation.reconnected]) {
    const subscription = spyOn(source, 'on').mockImplementation(() => () => {});
    restore.push(() => subscription.mockRestore());
  }
  return { get form() { return form as FormInstance; } };
}
const values = { title: 'new title', tag: 'fixture' };

test('two clicks during validation submit only once', async () => {
  const f = fixture(); const submitted = mock(async () => {});
  const v = render(<I18nextProvider i18n={i18n}><RequirementForm mode='create' onSubmit={submitted} onCancel={() => {}} /></I18nextProvider>);
  const validation = deferred<typeof values>();
  const validate = spyOn(f.form, 'validate').mockImplementation(() => validation.promise);
  restore.push(() => validate.mockRestore());
  fireEvent.click(v.getByText('requirements.form.submit'));
  fireEvent.click(v.getByText('requirements.form.submit'));
  await act(async () => { validation.resolve(values); });
  expect(validate).toHaveBeenCalledTimes(1);
  expect(submitted).toHaveBeenCalledTimes(1);
});

test('validation from a reset form cannot submit the former draft', async () => {
  const f = fixture(); const submitted = mock(async () => {});
  const form = (resetSignal: number) => <I18nextProvider i18n={i18n}><RequirementForm mode='create' resetSignal={resetSignal} onSubmit={submitted} onCancel={() => {}} /></I18nextProvider>;
  const v = render(form(0));
  const validation = deferred<typeof values>();
  const validate = spyOn(f.form, 'validate').mockImplementation(() => validation.promise);
  restore.push(() => validate.mockRestore());
  fireEvent.click(v.getByText('requirements.form.submit'));
  v.rerender(form(1));
  await act(async () => { validation.resolve(values); });
  expect(submitted).not.toHaveBeenCalled();
});

test('a current save consumes the returned requirement without a second GET', async () => {
  fixture(); const saved = mock(); const update = deferred<IRequirement>();
  const get = spyOn(ipcBridge.requirements.get, 'invoke').mockResolvedValue(item(1));
  const put = spyOn(ipcBridge.requirements.update, 'invoke').mockImplementation(() => update.promise);
  restore.push(() => get.mockRestore(), () => put.mockRestore());
  const v = render(<I18nextProvider i18n={i18n}><RequirementDrawer open mode='edit' requirementId={item(1).requirement_id} onClose={() => {}} onSaved={saved} /></I18nextProvider>);
  await act(async () => {});
  fireEvent.click(v.getByText('requirements.form.submit'));
  await act(async () => {});
  expect(put).toHaveBeenCalledTimes(1);
  await act(async () => { update.resolve({ ...item(1), title: 'saved response' }); });
  expect(get).toHaveBeenCalledTimes(1);
  expect(v.queryAllByText('saved response').length).toBeGreaterThan(0);
  expect(saved).toHaveBeenCalledTimes(1);
});

test('a save for a former target cannot replace or exit the new target editor', async () => {
  fixture(); const saved = mock(); const update = deferred<IRequirement>();
  const get = spyOn(ipcBridge.requirements.get, 'invoke').mockImplementation(async ({ requirement_id }) => requirement_id === item(1).requirement_id ? item(1) : item(2));
  const put = spyOn(ipcBridge.requirements.update, 'invoke').mockImplementation(() => update.promise);
  restore.push(() => get.mockRestore(), () => put.mockRestore());
  const drawer = (n: number) => <I18nextProvider i18n={i18n}><RequirementDrawer open mode='edit' requirementId={item(n).requirement_id} onClose={() => {}} onSaved={saved} /></I18nextProvider>;
  const v = render(drawer(1)); await act(async () => {});
  fireEvent.click(v.getByText('requirements.form.submit')); await act(async () => {});
  v.rerender(drawer(2)); await act(async () => {});
  await act(async () => { update.resolve(item(1)); });
  expect(get).toHaveBeenCalledTimes(2);
  expect(v.queryAllByText('requirements.form.submit').length).toBe(1);
  expect(saved).not.toHaveBeenCalled();
});

test('an old create cannot close a reopened drawer or clear its new submission state', async () => {
  const f = fixture(); const closed = mock(); const saved = mock();
  const pending = [deferred<IRequirement>(), deferred<IRequirement>()];
  let call = 0;
  const create = spyOn(ipcBridge.requirements.create, 'invoke').mockImplementation(() => pending[call++]!.promise);
  restore.push(() => create.mockRestore());
  const drawer = (open: boolean) => <I18nextProvider i18n={i18n}><RequirementDrawer open={open} mode='create' onClose={closed} onSaved={saved} /></I18nextProvider>;
  const v = render(drawer(true));
  act(() => f.form.setFieldsValue(values));
  fireEvent.click(v.getByText('requirements.form.submit')); await act(async () => {});
  v.rerender(drawer(false)); v.rerender(drawer(true));
  act(() => f.form.setFieldsValue(values));
  fireEvent.click(v.getByText('requirements.form.submit')); await act(async () => {});
  expect(create).toHaveBeenCalledTimes(2);
  await act(async () => { pending[0]!.resolve(item(1)); });
  expect(closed).not.toHaveBeenCalled(); expect(saved).not.toHaveBeenCalled();
  expect((v.getByText('requirements.form.submit').closest('button') as HTMLButtonElement).disabled).toBe(true);
  await act(async () => { pending[1]!.resolve(item(2)); });
  expect(closed).toHaveBeenCalledTimes(1); expect(saved).toHaveBeenCalledTimes(1);
});
