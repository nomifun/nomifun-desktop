import '../../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { useState } from 'react';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { IApiSshHost } from '@/common/adapter/ipcBridge';
import { parseSshHostId } from '@/common/types/ids';
import ssh from '@/renderer/services/i18n/locales/en-US/ssh.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';

const originalFetch = globalThis.fetch;
const originalWindowFetch = window.fetch;
const testFetch = (async () => new Response(JSON.stringify({ success: true, data: {} }), {
  headers: { 'Content-Type': 'application/json' },
})) as typeof fetch;
const installTransport = () => { globalThis.fetch = testFetch; window.fetch = testFetch; };
const restoreTransport = () => { globalThis.fetch = originalFetch; window.fetch = originalWindowFetch; };
installTransport();
const { ipcBridge } = await import('@/common');
const { SshHostFormModal } = await import('./SshHostManagement');
const { ThemeProvider } = await import('@/renderer/hooks/context/ThemeContext');
restoreTransport();
beforeEach(installTransport);
const restorers: Array<() => void> = [];
afterEach(() => { cleanup(); restorers.splice(0).forEach(restore => restore()); restoreTransport(); });

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US', resources: { 'en-US': { translation: { ssh, common } } },
  interpolation: { escapeValue: false },
});
const savedHost: IApiSshHost = {
  sshHostId: parseSshHostId('019f0000-0000-7000-8000-000000000001'),
  name: 'Saved host', host: 'saved.example.test', port: 22, username: 'deploy', authType: 'password',
  password: '***', privateKey: null, passphrase: null, certificate: null, sudoPassword: '***',
  hostFingerprint: null, createdAt: 1, updatedAt: 1,
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((complete, fail) => { resolve = complete; reject = fail; });
  return { promise, resolve, reject };
}

function mount(editHost?: IApiSshHost) {
  const pending = deferred<{ ok: boolean; message: string }>();
  const probe = spyOn(ipcBridge.ssh.testConnection, 'invoke').mockImplementation(() => pending.promise);
  const create = spyOn(ipcBridge.ssh.create, 'invoke').mockResolvedValue(savedHost);
  const update = spyOn(ipcBridge.ssh.update, 'invoke').mockResolvedValue(savedHost);
  restorers.push(() => probe.mockRestore(), () => create.mockRestore(), () => update.mockRestore());
  function Harness() {
    const [visible, setVisible] = useState(false);
    return <>
      <button onClick={() => setVisible(true)}>Open host form</button>
      <SshHostFormModal visible={visible} editHost={editHost} onClose={() => setVisible(false)} onSaved={() => {}} />
    </>;
  }
  const page = render(<I18nextProvider i18n={i18n}><ThemeProvider><Harness /></ThemeProvider></I18nextProvider>);
  const open = async () => {
    fireEvent.click(page.getByRole('button', { name: 'Open host form' }));
    await page.findByRole('heading', { name: editHost ? ssh.form.editTitle : ssh.form.addTitle });
    const name = page.getByLabelText(ssh.form.name) as HTMLInputElement;
    await waitFor(() => expect(name.value).toBe(editHost?.name ?? ''));
  };
  const field = (label: string) => page.getByLabelText(label) as HTMLInputElement;
  const change = (label: string, value: string) => fireEvent.change(field(label), { target: { value } });
  const fill = () => {
    change(ssh.form.name, 'Draft host');
    change(ssh.form.host, 'draft.example.test');
    change(ssh.form.username, 'draft-user');
    change(ssh.form.password, 'draft-password');
  };
  const testButton = () => page.getByRole('button', { name: ssh.form.test }) as HTMLButtonElement;
  const saveButton = () => page.getByRole('button', { name: ssh.form.save }) as HTMLButtonElement;
  return { page, pending, probe, create, update, open, field, change, fill, testButton, saveButton };
}

describe('SSH host form connection testing', () => {
  test('the add form tests the current draft without creating or updating a host', async () => {
    const view = mount();
    await view.open();
    expect(view.testButton()).toBeTruthy();
    view.fill();
    fireEvent.click(view.testButton());
    await waitFor(() => expect(view.probe).toHaveBeenCalledTimes(1));
    expect(view.probe.mock.calls[0]![0]).toMatchObject({
      host: 'draft.example.test', port: 22, username: 'draft-user', authType: 'password', password: 'draft-password',
    });
    const sent = JSON.parse(JSON.stringify(view.probe.mock.calls[0]![0]));
    expect(sent).not.toHaveProperty('sshHostId');
    expect(sent).not.toHaveProperty('name');
    expect(sent).not.toHaveProperty('sudoPassword');
    expect(view.create).not.toHaveBeenCalled();
    expect(view.update).not.toHaveBeenCalled();
    await act(async () => view.pending.resolve({ ok: true, message: 'connection succeeded' }));
    const result = await view.page.findByText(ssh.test.ok);
    expect(result.closest('.arco-message')).toBeNull();
    expect(view.page.getByRole('heading', { name: ssh.form.addTitle })).toBeTruthy();
    view.change(ssh.form.host, 'changed.example.test');
    await act(async () => {});
    expect(view.page.queryByText(ssh.test.ok)).toBeNull();
  });

  test('the edit form tests unsaved coordinates and keeps stored secrets out of the request', async () => {
    const view = mount(savedHost);
    await view.open();
    view.change(ssh.form.host, 'edited.example.test');
    view.change(ssh.form.username, 'edited-user');
    fireEvent.click(view.testButton());
    await waitFor(() => expect(view.probe).toHaveBeenCalledTimes(1));
    const request = view.probe.mock.calls[0]![0];
    expect(request).toMatchObject({ sshHostId: savedHost.sshHostId, host: 'edited.example.test', username: 'edited-user' });
    const sent = JSON.parse(JSON.stringify(request));
    expect(sent).not.toHaveProperty('password');
    expect(sent).not.toHaveProperty('sudoPassword');
    expect(JSON.stringify(request)).not.toContain('***');
    expect(view.create).not.toHaveBeenCalled();
    expect(view.update).not.toHaveBeenCalled();
    await act(async () => view.pending.resolve({ ok: false, message: 'authentication rejected' }));
    const result = await view.page.findByText(i18n.t('ssh.test.failed', { message: 'authentication rejected' }));
    expect(result.closest('.arco-message')).toBeNull();
  });

  test('invalid credentials never start a connection test', async () => {
    const view = mount();
    await view.open();
    view.fill();
    view.change(ssh.form.password, '');
    fireEvent.click(view.testButton());
    await act(async () => {});
    expect(view.probe).not.toHaveBeenCalled();
    expect(view.create).not.toHaveBeenCalled();
    expect(view.update).not.toHaveBeenCalled();
    expect(view.page.getByText(ssh.validation.passwordRequired)).toBeTruthy();
  });

  test('connection testing needs connection fields without requiring a display name', async () => {
    const view = mount();
    await view.open();
    view.fill();
    view.change(ssh.form.name, '');
    fireEvent.click(view.testButton());
    await waitFor(() => expect(view.probe).toHaveBeenCalledTimes(1));
    expect(view.create).not.toHaveBeenCalled();
    await act(async () => view.pending.resolve({ ok: true, message: 'connection succeeded' }));
    await view.page.findByText(ssh.test.ok);
  });

  test('a pending connection test prevents duplicate tests and saving', async () => {
    const view = mount();
    await view.open();
    view.fill();
    fireEvent.click(view.testButton());
    await waitFor(() => expect(view.probe).toHaveBeenCalledTimes(1));
    expect(view.saveButton().disabled).toBe(true);
    fireEvent.click(view.testButton());
    fireEvent.click(view.saveButton());
    await act(async () => {});
    expect(view.probe).toHaveBeenCalledTimes(1);
    expect(view.create).not.toHaveBeenCalled();
    await act(async () => view.pending.reject(new Error('test transport unavailable')));
    await view.page.findByText(i18n.t('ssh.test.failed', { message: 'Error: test transport unavailable' }));
    expect(view.saveButton().disabled).toBe(false);
  });

  test('a response for an older draft cannot report success for changed fields', async () => {
    const view = mount();
    await view.open();
    view.fill();
    fireEvent.click(view.testButton());
    await waitFor(() => expect(view.probe).toHaveBeenCalledTimes(1));
    view.change(ssh.form.host, 'newer.example.test');
    await act(async () => view.pending.resolve({ ok: true, message: 'connection succeeded' }));
    expect(view.page.queryByText(ssh.test.ok)).toBeNull();
    expect(view.field(ssh.form.host).value).toBe('newer.example.test');
  });

  test('opening animation cannot replace a draft after its connection test starts', async () => {
    const view = mount();
    await view.open();
    view.fill();
    fireEvent.click(view.testButton());
    await waitFor(() => expect(view.probe).toHaveBeenCalledTimes(1));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 450)); });
    await act(async () => view.pending.resolve({ ok: true, message: 'connection succeeded' }));
    expect(view.field(ssh.form.host).value).toBe('draft.example.test');
    await view.page.findByText(ssh.test.ok);
  });

  test('an edit form has its saved values before the opening transition completes', async () => {
    const view = mount(savedHost);
    fireEvent.click(view.page.getByRole('button', { name: 'Open host form' }));
    expect(view.field(ssh.form.host).value).toBe(savedHost.host);
    expect(view.field(ssh.form.password).value).toBe('***');
    view.change(ssh.form.host, 'immediate-edit.example.test');
    fireEvent.click(view.testButton());
    await waitFor(() => expect(view.probe).toHaveBeenCalledTimes(1));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 450)); });
    await act(async () => view.pending.resolve({ ok: true, message: 'connection succeeded' }));
    expect(view.field(ssh.form.host).value).toBe('immediate-edit.example.test');
    await view.page.findByText(ssh.test.ok);
  });

  test('a response arriving after closing cannot populate a reopened form', async () => {
    const view = mount();
    await view.open();
    view.fill();
    fireEvent.click(view.testButton());
    await waitFor(() => expect(view.probe).toHaveBeenCalledTimes(1));
    fireEvent.click(view.page.getByRole('button', { name: ssh.form.cancel }));
    // Arco keeps the hidden modal mounted until its 400ms close transition ends.
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 450)); });
    await view.open();
    await act(async () => view.pending.resolve({ ok: true, message: 'connection succeeded' }));
    expect(view.page.queryByText(ssh.test.ok)).toBeNull();
    expect(view.field(ssh.form.host).value).toBe('');
    expect(view.saveButton().disabled).toBe(false);
  });

  test('immediately reopening during the closing transition keeps the new draft', async () => {
    const view = mount();
    await view.open();
    view.fill();
    fireEvent.click(view.page.getByRole('button', { name: ssh.form.cancel }));
    await view.open();
    expect(view.field(ssh.form.host).value).toBe('');
    view.change(ssh.form.name, 'Reopened draft');
    view.change(ssh.form.host, 'reopened.example.test');
    view.change(ssh.form.username, 'reopened-user');
    view.change(ssh.form.password, 'reopened-password');
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 450)); });
    expect(view.field(ssh.form.host).value).toBe('reopened.example.test');
    fireEvent.click(view.testButton());
    await waitFor(() => expect(view.probe).toHaveBeenCalledTimes(1));
    expect(view.probe.mock.calls[0]![0]).toMatchObject({
      host: 'reopened.example.test', username: 'reopened-user', password: 'reopened-password',
    });
    await act(async () => view.pending.resolve({ ok: true, message: 'connection succeeded' }));
    await view.page.findByText(ssh.test.ok);
  });
});
