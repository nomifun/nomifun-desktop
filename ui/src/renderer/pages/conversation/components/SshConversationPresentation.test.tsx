import '../../../../../test/setup-dom.ts';
import { act, cleanup, render, waitFor } from '@testing-library/react';
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
  hostFingerprint: null, status: 'connected', lastConnectedAt: null, createdAt: 1000, updatedAt: 1000,
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
    expect((secondPill as HTMLButtonElement).disabled).toBe(true);
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
