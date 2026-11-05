import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import type { ReactNode } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { IApiSshHost, IApiSshStatus } from '@/common/adapter/ipcBridge';
import type { TChatConversation } from '@/common/config/storage';
import { parseConversationId, parseEntityId } from '@/common/types/ids';
import { firstSshHostId, secondSshHostId, missingSshHostId, makeSshConversation } from '../../../../../test/fixtures/sshConversation';
import sshLocale from '@/renderer/services/i18n/locales/en-US/ssh.json';
import SshHostStatusPill from './SshHostStatusPill';
import SshSessionGroup from '../SessionList/SshSessionGroup';
import { useConversationListSync } from '../SessionList/hooks/useConversationListSync';

afterEach(cleanup);

const locale = createInstance();
await locale.use(initReactI18next).init({ lng: 'en', initImmediate: false,
  resources: { en: { translation: { ssh: sshLocale } } } });

const host = (sshHostId: IApiSshHost['sshHostId'], name: string): IApiSshHost => ({
  sshHostId, name, host: 'example.test', port: 22, username: 'rika', authType: 'password',
  password: null, privateKey: null, passphrase: null, certificate: null, sudoPassword: null,
  hostFingerprint: null, createdAt: 1000, updatedAt: 1000,
});

function Sidebar() {
  const { conversations } = useConversationListSync();
  return <>
    <div data-testid='ordinary-sessions'>{conversations.map((item) => <span key={item.id}>{item.name}</span>)}</div>
    <div data-testid='ssh-sessions'><SshSessionGroup activeConversationId={null}
      renderRow={(item) => <span key={item.id}>{item.name}</span>} /></div>
  </>;
}

test('canonical SSH binding drives the header and sidebar through host switches, missing hosts and removal', async () => {
  let conversation = makeSshConversation();
  const local: TChatConversation = { ...makeSshConversation(null),
    id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000078'), name: 'Local work' };
  let items = [conversation, local];
  let agentChanged: Parameters<typeof ipcBridge.agentPlatform.sessions.onAgentChanged.on>[0] | undefined;
  const statusListeners = new Set<Parameters<typeof ipcBridge.ssh.onStatus.on>[0]>();
  const firstStatus: IApiSshStatus = {
    conversationId: conversation.id, sshHostId: firstSshHostId, state: 'connected', attempt: 0,
    nextRetryInMs: null, hostFingerprint: null, detail: null, reaped: null, retryable: null, changedAt: 1000,
  };
  let pendingSnapshot: Promise<IApiSshStatus[]> | undefined;
  const list = spyOn(ipcBridge.database.getUserConversations, 'invoke').mockImplementation(async () => ({
    items, total: items.length, has_more: false, next_cursor: null,
  }));
  const statuses = spyOn(ipcBridge.ssh.statuses, 'invoke').mockImplementation(() => pendingSnapshot ?? Promise.resolve([firstStatus]));
  const statusEvents = spyOn(ipcBridge.ssh.onStatus, 'on').mockImplementation((callback) => {
    statusListeners.add(callback);
    return () => { statusListeners.delete(callback); };
  });
  const subscriptions = [
    spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockImplementation((callback) => {
      agentChanged = callback;
      return () => {};
    }),
    spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {}),
    spyOn(ipcBridge.conversation.turnPaused, 'on').mockReturnValue(() => {}),
    spyOn(ipcBridge.conversation.listChanged, 'on').mockReturnValue(() => {}),
    spyOn(ipcBridge.conversation.turnStarted, 'on').mockReturnValue(() => {}),
    spyOn(ipcBridge.conversation.responseStream, 'on').mockReturnValue(() => {}),
    spyOn(ipcBridge.conversation.turnCompleted, 'on').mockReturnValue(() => {}),
  ];
  const cache = new Map();
  const wrapper = ({ children }: { children: ReactNode }) => <I18nextProvider i18n={locale}>
    <SWRConfig value={{ provider: () => cache, revalidateOnMount: false,
      fallback: { 'ssh-hosts.list': [host(firstSshHostId, 'Ubuntu A'), host(secondSshHostId, 'Ubuntu B')] } }}>
      {children}
    </SWRConfig>
  </I18nextProvider>;
  const app = () => <><SshHostStatusPill conversation={conversation} /><Sidebar /></>;
  const refresh = async () => {
    items = [conversation, local];
    await act(async () => {
      agentChanged?.({ agent_session_id: parseEntityId('agent-session', conversation.id), transition_id: 'fixture-transition',
        previous_agent_label: 'SSH Agent', current_agent_label: 'SSH Agent',
        binding_version: conversation.agent_snapshot!.canonical_binding!.binding_version, effective_from: 'next_turn' });
    });
  };

  try {
    const view = render(app(), { wrapper });
    await waitFor(() => expect(view.getByTestId('ssh-sessions').textContent).toContain('Ubuntu A'));
    expect(view.getByTestId('ordinary-sessions').textContent).toBe('Local work');
    expect(view.getByTestId('ssh-sessions').textContent).toContain('Remote work');
    await waitFor(() => expect(view.getByTestId('ssh-host-status-pill').className).toContain('--active'));
    expect(view.getByTestId('ssh-host-status-pill').textContent).toBe('Ubuntu A');

    let resolveSnapshot!: (rows: IApiSshStatus[]) => void;
    pendingSnapshot = new Promise((resolve) => { resolveSnapshot = resolve; });
    conversation = makeSshConversation(secondSshHostId, 2);
    view.rerender(app());
    await refresh();
    await waitFor(() => expect(view.getByTestId('ssh-sessions').textContent).toContain('Ubuntu B'));
    expect(view.getByTestId('ssh-sessions').textContent).not.toContain('Ubuntu A');
    const secondPill = view.getByTestId('ssh-host-status-pill');
    expect(secondPill.textContent).toBe('Ubuntu B');
    expect(secondPill.className).toContain('--inactive');
    expect((secondPill as HTMLButtonElement).disabled).toBe(false);
    await act(async () => {
      for (const listener of statusListeners) listener(firstStatus);
      resolveSnapshot([{ ...firstStatus, sshHostId: secondSshHostId, changedAt: 10 }]);
    });
    await waitFor(() => expect(view.getByTestId('ssh-host-status-pill').className).toContain('--active'));
    expect((view.getByTestId('ssh-host-status-pill') as HTMLButtonElement).disabled).toBe(false);

    conversation = makeSshConversation(missingSshHostId, 3);
    view.rerender(app());
    await refresh();
    await waitFor(() => expect(view.getByTestId('ssh-sessions').textContent).toContain('Deleted host'));
    expect(view.getByTestId('ssh-sessions').textContent).toContain('Remote work');
    expect(view.getByTestId('ssh-host-status-pill').textContent).toContain(`Deleted host · ${missingSshHostId.slice(0, 8)}`);

    conversation = makeSshConversation(null, 4);
    view.rerender(app());
    await refresh();
    await waitFor(() => expect(view.getByTestId('ordinary-sessions').textContent).toContain('Remote work'));
    expect(view.getByTestId('ssh-sessions').textContent).toBe('');
    expect(view.queryByTestId('ssh-host-status-pill')).toBeNull();
    expect(statusListeners.size).toBe(0);
  } finally {
    cleanup();
    list.mockRestore();
    statuses.mockRestore();
    statusEvents.mockRestore();
    subscriptions.forEach((subscription) => subscription.mockRestore());
  }
});

test('link controls distinguish loading, failed status reads and no link, then connect and disconnect the canonical session', async () => {
  const conversation = makeSshConversation();
  let rejectSnapshot!: (error: Error) => void;
  const pending = new Promise<IApiSshStatus[]>((_, reject) => { rejectSnapshot = reject; });
  let rows: IApiSshStatus[] = [];
  const listeners = new Set<Parameters<typeof ipcBridge.ssh.onStatus.on>[0]>();
  const statuses = spyOn(ipcBridge.ssh.statuses, 'invoke').mockImplementationOnce(() => pending)
    .mockImplementation(async () => rows);
  const events = spyOn(ipcBridge.ssh.onStatus, 'on').mockImplementation((callback) => {
    listeners.add(callback);
    return () => { listeners.delete(callback); };
  });
  const reconnected = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const connect = spyOn(ipcBridge.agentPlatform.sessions.connectSsh, 'invoke').mockImplementation(async () => {
    for (const listener of listeners) listener(rows[0]!);
    return { accepted: true };
  });
  const disconnect = spyOn(ipcBridge.agentPlatform.sessions.disconnectSsh, 'invoke').mockImplementation(async () => {
    const closed: IApiSshStatus = { ...rows[0]!, state: 'closed', reaped: false, changedAt: 3000 };
    rows = [];
    for (const listener of listeners) listener(closed);
    return { accepted: true };
  });
  const wrapper = ({ children }: { children: ReactNode }) => <I18nextProvider i18n={locale}>
    <SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false,
      fallback: { 'ssh-hosts.list': [host(firstSshHostId, 'Ubuntu A')] } }}>{children}</SWRConfig>
  </I18nextProvider>;

  try {
    const view = render(<SshHostStatusPill conversation={conversation} />, { wrapper });
    fireEvent.click(view.getByTestId('ssh-host-status-pill'));
    await waitFor(() => expect(view.getByText('Loading link status…')).toBeTruthy());
    expect(connect).not.toHaveBeenCalled();
    expect(view.queryByText(sshLocale.pill.noLink)).toBeNull();
    await act(async () => { rejectSnapshot(new Error('status service unavailable')); });
    await waitFor(() => expect(view.getByText('Link status unavailable')).toBeTruthy());
    expect(view.getByRole('alert').textContent).toContain('status service unavailable');
    expect(view.queryByText(sshLocale.pill.noLink)).toBeNull();

    fireEvent.click(view.getByText('Refresh status'));
    await waitFor(() => expect(view.getByText(sshLocale.pill.noLink)).toBeTruthy());
    expect((view.getByTestId('ssh-link-connect') as HTMLButtonElement).disabled).toBe(false);
    rows = [{ conversationId: conversation.id, sshHostId: firstSshHostId, state: 'connecting', attempt: 1,
      nextRetryInMs: null, hostFingerprint: null, detail: null, reaped: null, retryable: null, changedAt: 1000 }];
    fireEvent.click(view.getByTestId('ssh-link-connect'));
    await waitFor(() => expect(connect).toHaveBeenCalledWith({ agent_session_id: conversation.id }));
    await waitFor(() => expect(view.getByText('Connecting…')).toBeTruthy());
    await act(async () => {
      rows = [{ ...rows[0]!, state: 'connected', attempt: 0, changedAt: 2000 }];
      for (const listener of listeners) listener(rows[0]!);
    });
    await waitFor(() => expect(view.getByTestId('ssh-host-status-pill').className).toContain('--active'));
    fireEvent.click(view.getByTestId('ssh-link-disconnect'));
    await waitFor(() => expect(disconnect).toHaveBeenCalledWith({ agent_session_id: conversation.id }));
    await waitFor(() => expect(view.getByText('Closed')).toBeTruthy());
    expect(view.getByText(sshLocale.pill.unconfirmedExit)).toBeTruthy();
    expect(statuses).toHaveBeenCalledTimes(2);
    expect(view.getByTestId('ssh-link-connect').textContent).toBe('Reconnect');

    rows = [{ conversationId: conversation.id, sshHostId: firstSshHostId, state: 'dropped', attempt: 0,
      nextRetryInMs: null, hostFingerprint: null, detail: 'SSH authentication rejected', reaped: null,
      retryable: false, changedAt: 4000 }];
    fireEvent.click(view.getByTestId('ssh-link-connect'));
    await waitFor(() => expect(view.getByText('SSH authentication rejected')).toBeTruthy());
    expect(view.getByText(sshLocale.pill.droppedActionRequired)).toBeTruthy();
    expect(view.queryByText(sshLocale.pill.unconfirmedExit)).toBeNull();

    connect.mockRejectedValueOnce(new Error('host credential rejected'));
    fireEvent.click(view.getByTestId('ssh-link-connect'));
    await waitFor(() => expect(view.getByRole('alert').textContent).toContain('host credential rejected'));
    expect(view.getByText('Link lost')).toBeTruthy();
  } finally {
    cleanup();
    statuses.mockRestore();
    events.mockRestore();
    reconnected.mockRestore();
    connect.mockRestore();
    disconnect.mockRestore();
  }
});

test('an empty in-flight snapshot cannot clear a newer live link event', async () => {
  const conversation = makeSshConversation();
  let finish!: (rows: IApiSshStatus[]) => void;
  let listener: Parameters<typeof ipcBridge.ssh.onStatus.on>[0] | undefined;
  const statuses = spyOn(ipcBridge.ssh.statuses, 'invoke').mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  const events = spyOn(ipcBridge.ssh.onStatus, 'on').mockImplementation((callback) => { listener = callback; return () => {}; });
  const reconnected = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const wrapper = ({ children }: { children: ReactNode }) => <I18nextProvider i18n={locale}>
    <SWRConfig value={{ provider: () => new Map(), revalidateOnMount: false,
      fallback: { 'ssh-hosts.list': [host(firstSshHostId, 'Ubuntu A')] } }}>{children}</SWRConfig>
  </I18nextProvider>;
  try {
    const view = render(<SshHostStatusPill conversation={conversation} />, { wrapper });
    await act(async () => {
      listener?.({ conversationId: conversation.id, sshHostId: firstSshHostId, state: 'connected', attempt: 0,
        nextRetryInMs: null, hostFingerprint: null, detail: null, reaped: null, retryable: null, changedAt: 1000 });
      finish([]);
    });
    await waitFor(() => expect(view.getByTestId('ssh-host-status-pill').className).toContain('--active'));
  } finally {
    cleanup();
    statuses.mockRestore();
    events.mockRestore();
    reconnected.mockRestore();
  }
});
