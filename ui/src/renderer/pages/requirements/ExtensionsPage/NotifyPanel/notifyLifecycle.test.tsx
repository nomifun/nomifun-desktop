import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { Component, type ReactNode } from 'react';
import { Form, Message, type FormInstance } from '@arco-design/web-react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { ITagSetting, IWebhook } from '@/common/adapter/ipcBridge';
import { parseWebhookId } from '@/common/types/ids';
import ChannelFormModal from './ChannelFormModal';
import ChannelList from './ChannelList';
import RoutingRuleList from './RoutingRuleList';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });
const restore: Array<() => void> = [];
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(fn => fn()); });
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
const channel: IWebhook = { webhook_id: parseWebhookId('019b0000-0000-7000-8000-000000000001'), name: 'channel', platform: 'http', url: 'https://example.invalid', description: '', enabled: true, has_secret: false, created_at: 1, updated_at: 1 };
const setting = (tag = 'fixture'): ITagSetting => ({ tag, description: '', webhook_id: null, notify_events: ['done'] });
const tagSummary = (tag: string) => ({ tag, pending: 1, in_progress: 0, done: 0, failed: 0, cancelled: 0, needs_review: 0, total: 1, paused: false });
const provider = (child: ReactNode) => <I18nextProvider i18n={i18n}>{child}</I18nextProvider>;
function fixture() {
  let captured: unknown;
  const original = Form.useForm;
  const formSpy = spyOn(Form, 'useForm').mockImplementation(existing => { const result = original(existing); captured = result[0]; return result; });
  const success = mock(() => () => {}); const error = mock(() => () => {});
  const messages = spyOn(Message, 'useMessage').mockReturnValue([{ success, error }, <></>]);
  const tags = spyOn(ipcBridge.requirements.tags, 'invoke').mockResolvedValue([tagSummary('fixture')]);
  const get = spyOn(ipcBridge.webhook.getTagSetting, 'invoke').mockImplementation(async ({ tag }) => setting(tag));
  restore.push(() => formSpy.mockRestore(), () => messages.mockRestore(), () => tags.mockRestore(), () => get.mockRestore());
  return { get form() { return captured as FormInstance; }, success, error, tags, get };
}
const values = { name: 'channel', platform: 'http', url: 'https://example.invalid', enabled: true };
const confirm = (v: ReturnType<typeof render>) => v.baseElement.querySelector<HTMLButtonElement>('.arco-modal-footer .arco-btn-primary')!;
class Boundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() { return this.state.failed ? <div>render failed</div> : this.props.children; }
}

test('tag setting load errors stay visible and retry instead of becoming defaults', async () => {
  const f = fixture(); f.get.mockRejectedValueOnce(new Error('offline'));
  const v = render(provider(<RoutingRuleList channels={[]} />)); await act(async () => {});
  expect(f.error).toHaveBeenCalledTimes(1);
  expect(v.queryAllByRole('checkbox')).toHaveLength(0);
  fireEvent.click(v.getByText('requirements.retry')); await act(async () => {});
  expect(v.getAllByRole('checkbox')).toHaveLength(3);
});

test('special tag keys are safe before and after settings load and an update', async () => {
  const f = fixture(); const pending = deferred<ITagSetting>();
  f.tags.mockResolvedValue([tagSummary('__proto__'), tagSummary('constructor')]);
  f.get.mockImplementation(({ tag }) => tag === '__proto__' ? pending.promise : Promise.resolve(setting(tag)));
  const put = spyOn(ipcBridge.webhook.setTagSetting, 'invoke').mockResolvedValue(setting('__proto__')); restore.push(() => put.mockRestore());
  const v = render(provider(<Boundary><RoutingRuleList channels={[]} /></Boundary>));
  await act(async () => {}); const failedDuringLoad = v.queryByText('render failed') != null;
  await act(async () => { pending.resolve(setting('__proto__')); });
  expect(failedDuringLoad).toBe(false);
  fireEvent.click(v.getAllByRole('checkbox')[1]!); await act(async () => {});
  expect(v.queryByText('render failed')).toBeNull(); expect(v.getAllByRole('checkbox')).toHaveLength(6);
});

test('a pending rule update prevents another full-DTO response from racing it', async () => {
  fixture(); const pending = deferred<ITagSetting>();
  const put = spyOn(ipcBridge.webhook.setTagSetting, 'invoke').mockImplementation(() => pending.promise); restore.push(() => put.mockRestore());
  const v = render(provider(<RoutingRuleList channels={[]} />)); await act(async () => {});
  fireEvent.click(v.getAllByRole('checkbox')[1]!); fireEvent.click(v.getAllByRole('checkbox')[2]!);
  const count = put.mock.calls.length;
  await act(async () => { pending.resolve({ ...setting(), notify_events: ['done', 'failed'] }); });
  expect(count).toBe(1); expect((v.getAllByRole('checkbox')[1] as HTMLInputElement).checked).toBe(true);
});

test('channel modal blocks duplicate validation and ignores a reset draft', async () => {
  const f = fixture(); const pending = deferred<typeof values>();
  const create = spyOn(ipcBridge.webhook.create, 'invoke').mockResolvedValue(channel); restore.push(() => create.mockRestore());
  const modal = (visible: boolean) => provider(<ChannelFormModal visible={visible} editing={null} onClose={() => {}} onSuccess={() => {}} />);
  const v = render(modal(true));
  const validate = spyOn(f.form, 'validate').mockImplementation(() => pending.promise); restore.push(() => validate.mockRestore());
  fireEvent.click(confirm(v)); fireEvent.click(confirm(v)); const count = validate.mock.calls.length;
  v.rerender(modal(false)); v.rerender(modal(true)); await act(async () => { pending.resolve(values); });
  expect(count).toBe(1); expect(create).not.toHaveBeenCalled();
});

test('old channel creation cannot report success or close a reopened modal', async () => {
  const f = fixture(); const pending = deferred<IWebhook>(); const onSuccess = mock();
  const create = spyOn(ipcBridge.webhook.create, 'invoke').mockImplementation(() => pending.promise); restore.push(() => create.mockRestore());
  const modal = (visible: boolean) => provider(<ChannelFormModal visible={visible} editing={null} onClose={() => {}} onSuccess={onSuccess} />);
  const v = render(modal(true)); act(() => f.form.setFieldsValue(values));
  fireEvent.click(confirm(v)); await act(async () => {}); expect(create).toHaveBeenCalledTimes(1);
  v.rerender(modal(false)); v.rerender(modal(true)); await act(async () => { pending.resolve(channel); });
  expect(onSuccess).not.toHaveBeenCalled(); expect(f.success).not.toHaveBeenCalled();
});

test('a channel refresh after save cannot close a newly opened editor', async () => {
  const f = fixture(); const pending = deferred<void>(); const reload = mock(() => pending.promise);
  const create = spyOn(ipcBridge.webhook.create, 'invoke').mockResolvedValue(channel); restore.push(() => create.mockRestore());
  const v = render(provider(<ChannelList channels={[channel]} loading={false} error={null} reloadChannels={reload} />));
  fireEvent.click(v.getByText('requirements.notify.newChannel')); act(() => f.form.setFieldsValue(values));
  fireEvent.click(confirm(v)); await act(async () => {}); expect(reload).toHaveBeenCalledTimes(1);
  fireEvent.click(v.getByText('webhook.actions.edit')); await act(async () => {});
  await act(async () => { pending.resolve(); }); expect(v.queryByText('webhook.form.editTitle')).not.toBeNull();
});
