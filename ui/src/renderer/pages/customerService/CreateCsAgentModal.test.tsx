import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { Form, Message, type FormInstance } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { ICsAgent, ICsAgentPatch } from '@/common/adapter/ipcBridge';
import type { IProvider } from '@/common/config/storage';
import { parseCsAgentId, parseProviderId } from '@/common/types/ids';
import CreateCsAgentModal from './CreateCsAgentModal';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
const restore: Array<() => void> = [];
afterEach(() => {
  cleanup();
  restore.splice(0).reverse().forEach(dispose => dispose());
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const agent: ICsAgent = {
  cs_agent_id: parseCsAgentId('019b0000-0000-7000-8000-000000000001'),
  name: 'Support', greeting: '', persona: '', service_policy: '', provider_id: null, model: null,
  knowledge_base_ids: [], enabled: true, max_concurrent: 8, audit_retention_days: 30,
  created_at: 1, updated_at: 1,
};
const providerId = parseProviderId('019b0000-0000-7000-8000-000000000002');
const provider: IProvider = {
  id: providerId, name: 'Fixture provider', platform: 'test', base_url: 'https://example.test',
  auth_scheme: 'none', has_credentials: false, enabled: true,
  models: [{
    provider_id: providerId, model: 'fixture-chat', enabled: true, sort_order: 0,
    created_at: 1, updated_at: 1,
    capabilities: [{ task: 'chat', traits: [], protocol: 'test.chat', connection_role: 'default',
      allow_cross_origin_credentials: false, provider_params: {}, created_at: 1, updated_at: 1 }],
  }],
};

function fixture() {
  let form: unknown;
  const original = Form.useForm;
  const useForm = spyOn(Form, 'useForm').mockImplementation(existing => {
    const result = original(existing);
    form = result[0];
    return result;
  });
  const providers = spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([provider]);
  const bases = spyOn(ipcBridge.knowledge.listBases, 'invoke').mockResolvedValue([]);
  const success = spyOn(Message, 'success').mockImplementation(() => () => {});
  const error = spyOn(Message, 'error').mockImplementation(() => () => {});
  restore.push(() => useForm.mockRestore(), () => providers.mockRestore(), () => bases.mockRestore(),
    () => success.mockRestore(), () => error.mockRestore());
  const create = mock(async (_input: { name: string } & ICsAgentPatch) => agent);
  const closed = mock();
  const created = mock();
  const swr = { provider: () => new Map(), dedupingInterval: 0 };
  const modal = (visible: boolean) => (
    <SWRConfig value={swr}><I18nextProvider i18n={i18n}>
      <CreateCsAgentModal visible={visible} create={create} onClose={closed} onCreated={created} />
    </I18nextProvider></SWRConfig>
  );
  const view = render(modal(true));
  return {
    view, create, closed, created, success, error,
    get form() { return form as FormInstance; },
    setVisible: (visible: boolean) => view.rerender(modal(visible)),
    submit: () => fireEvent.click(view.baseElement.querySelector('.arco-modal-footer .arco-btn-primary')!),
  };
}

test('invalid names stay in the form, and a failed create preserves the draft for retry', async () => {
  const f = fixture();
  for (const name of ['   ', '']) {
    act(() => f.form.setFieldValue('name', name));
    f.submit();
    await act(async () => {});
    expect(f.create).not.toHaveBeenCalled();
    expect(f.view.queryByText('请输入客服名称')).not.toBeNull();
    expect(f.error).not.toHaveBeenCalled();
  }
  f.create.mockRejectedValueOnce(new Error('offline'));
  act(() => f.form.setFieldValue('name', '  Support  '));
  f.submit();
  await act(async () => {});
  expect(f.error).toHaveBeenCalledWith('offline');
  expect(f.form.getFieldValue('name')).toBe('  Support  ');
  f.submit();
  await act(async () => {});
  expect(f.create).toHaveBeenLastCalledWith({ name: 'Support', greeting: '', persona: '', service_policy: '',
    provider_id: null, model: null, knowledge_base_ids: [], max_concurrent: 8 });
  expect(f.created).toHaveBeenCalledWith(agent);
});

test('two clicks during validation create only one agent', async () => {
  const f = fixture();
  const validation = deferred<{ name: string }>();
  const validate = spyOn(f.form, 'validate').mockImplementation(() => validation.promise);
  restore.push(() => validate.mockRestore());
  f.submit();
  f.submit();
  await act(async () => { validation.resolve({ name: 'Support' }); });
  expect(validate).toHaveBeenCalledTimes(1);
  expect(f.create).toHaveBeenCalledTimes(1);
});

test('validation from a closed modal cannot submit after reopening', async () => {
  const f = fixture();
  const validation = deferred<{ name: string }>();
  const validate = spyOn(f.form, 'validate').mockImplementation(() => validation.promise);
  restore.push(() => validate.mockRestore());
  f.submit();
  f.setVisible(false);
  f.setVisible(true);
  await act(async () => { validation.resolve({ name: 'former draft' }); });
  expect(f.create).not.toHaveBeenCalled();
  expect(f.closed).not.toHaveBeenCalled();
});

test('old create success cannot reset or close a reopened modal with a newer request', async () => {
  const f = fixture();
  const first = deferred<ICsAgent>();
  const second = deferred<ICsAgent>();
  f.create.mockImplementationOnce(() => first.promise).mockImplementationOnce(() => second.promise);
  act(() => f.form.setFieldValue('name', 'first'));
  f.submit();
  await act(async () => {});
  f.setVisible(false);
  f.setVisible(true);
  act(() => f.form.setFieldValue('name', 'second'));
  f.submit();
  await act(async () => {});
  const callsBeforeOldCompletion = f.create.mock.calls.length;
  await act(async () => { first.resolve(agent); });
  const draftAfterOldCompletion = f.form.getFieldValue('name');
  const closesAfterOldCompletion = f.closed.mock.calls.length;
  const successesAfterOldCompletion = f.success.mock.calls.length;
  const busyAfterOldCompletion = f.view.baseElement.querySelector('.arco-modal-footer .arco-btn-primary')!.classList.contains('arco-btn-loading');
  await act(async () => { second.resolve({ ...agent, name: 'second' }); });
  expect(draftAfterOldCompletion).toBe('second');
  expect(callsBeforeOldCompletion).toBe(2);
  expect(closesAfterOldCompletion).toBe(0);
  expect(successesAfterOldCompletion).toBe(0);
  expect(busyAfterOldCompletion).toBe(true);
  expect(f.created).toHaveBeenCalledTimes(1);
  expect(f.created).toHaveBeenCalledWith({ ...agent, name: 'second' });
});

test('create failure after unmount does not publish a stale error', async () => {
  const f = fixture();
  const pending = deferred<ICsAgent>();
  f.create.mockImplementation(() => pending.promise);
  act(() => f.form.setFieldValue('name', 'Support'));
  f.submit();
  await act(async () => {});
  f.view.unmount();
  await act(async () => { pending.reject(new Error('late failure')); });
  expect(f.error).not.toHaveBeenCalled();
  expect(f.closed).not.toHaveBeenCalled();
  expect(f.created).not.toHaveBeenCalled();
});

test('provider form resets also clear its model options', async () => {
  const f = fixture();
  await act(async () => {});
  const selects = () => f.view.baseElement.querySelectorAll('.arco-select');
  fireEvent.click(selects()[0]!);
  fireEvent.click(await f.view.findByText('Fixture provider'));
  fireEvent.click(selects()[1]!);
  expect(await f.view.findByText('fixture-chat')).not.toBeNull();
  fireEvent.click(f.view.getByText('fixture-chat'));
  act(() => f.form.resetFields());
  fireEvent.click(selects()[1]!);
  await act(async () => {});
  expect(f.form.getFieldValue('provider_id')).toBeUndefined();
  expect(f.view.queryAllByText('fixture-chat').length).toBe(0);
});

test('concurrency input cannot send a fractional value to the integer API', async () => {
  const f = fixture();
  act(() => f.form.setFieldValue('name', 'Support'));
  const input = f.view.baseElement.querySelector('.arco-input-number input')!;
  fireEvent.change(input, { target: { value: '2.5' } });
  fireEvent.blur(input);
  f.submit();
  await act(async () => {});
  expect(f.create).toHaveBeenCalledTimes(1);
  expect(Number.isInteger(f.create.mock.calls[0]![0].max_concurrent)).toBe(true);
});
