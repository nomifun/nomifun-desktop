import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { Link, MemoryRouter, Route, Routes } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { createStoredMessageMapper } from '@/common/adapter/storedMessageMapper';
import { configService } from '@/common/config/configService';
import type { TChatConversation } from '@/common/config/storage';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { PreviewProvider } from '../Preview';
import NomiChat from '../platforms/nomi/NomiChat';
import type { NomiModelSelection } from '../platforms/nomi/useNomiModelSelection';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: {
  'en-US': { translation: { messages: messagesLocale, conversation: conversationLocale } },
} });
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000961');
const rootId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000962');
const messageId = (index: number) => parseMessageId(`0190f5fe-7c00-7a00-8000-${String(index).padStart(12, '0')}`);
const selection: NomiModelSelection = {
  providers: [], pickerDisabled: true, getAvailableModels: () => [],
  handleSelectModel: async () => {}, getDisplayModelName: name => name ?? '',
};
afterEach(() => { cleanup(); mock.restore(); });

test('real route remount restores compact active thinking before runtime hydration and keeps phase disclosures current', async () => {
  spyOn(configService, 'get').mockImplementation((key =>
    key === 'chat.thinking.contentLength' ? 'compact' : undefined) as typeof configService.get);
  spyOn(configService, 'subscribe').mockImplementation(() => () => {});
  for (const name of ['turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockImplementation(() => () => {});
  }
  const reconnectListeners = new Set<() => void>();
  spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(listener => {
    const reconnect = () => listener();
    reconnectListeners.add(reconnect);
    return () => reconnectListeners.delete(reconnect);
  });
  let stream: Parameters<typeof ipcBridge.conversation.responseStream.on>[0] | undefined;
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => {
    stream = listener;
    return () => { if (stream === listener) stream = undefined; };
  });
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockImplementation(() => () => {});
  spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);

  const startAt = Date.now() - 12_000;
  const authority = { id: conversationId, type: 'nomi', status: 'running', runtime: {
    state: 'running', can_send_message: false, has_runtime: true, is_processing: true,
    active_turn_id: rootId, processing_started_at: startAt,
  } } as TChatConversation;
  let deferRuntime = true;
  const pendingRuntimeReads: Array<(value: TChatConversation) => void> = [];
  spyOn(ipcBridge.conversation.get, 'invoke').mockImplementation(() => deferRuntime
    ? new Promise(resolve => pendingRuntimeReads.push(resolve)) : Promise.resolve(authority));
  type HistoryPage = Awaited<ReturnType<typeof ipcBridge.database.getConversationMessages.invoke>>;
  const pendingHistoryReads: Array<(page: HistoryPage) => void> = [];
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementation(() =>
    new Promise(resolve => pendingHistoryReads.push(resolve)));
  let renderKey = 0;
  const mapStored = createStoredMessageMapper(() => `history-row-${++renderKey}`);
  const row = (id: number, type: TMessage['type'], content: unknown, position = 'left') => mapStored({
    message_id: messageId(id), msg_id: id === 3 ? rootId : messageId(id),
    conversation_id: conversationId, type, position, hidden: false, created_at: startAt + id,
    content: { ...(content as object), turn_id: rootId },
  });
  const canonicalPage = (currentStatus: 'thinking' | 'done' = 'thinking') => [
    row(4, 'thinking', { content: 'Current reasoning body.', status: currentStatus }),
    // This is the actual REST shape that used to be interpreted as a terminal.
    row(3, 'agent_status', { backend: 'nomi', status: 'prepared', turn_summary: true,
      turn_state: 'running', started_at_ms: startAt, finished_at_ms: null, finished_seq: null }),
    row(2, 'thinking', { content: 'Earlier reasoning body.', status: 'done' }),
    row(1, 'text', { content: 'Inspect the current task' }, 'right'),
  ];
  const replyHistory = async (index: number, status: 'thinking' | 'done' = 'thinking') => {
    expect(pendingHistoryReads[index]).toBeDefined();
    await act(async () => pendingHistoryReads[index]({ items: canonicalPage(status), has_more: false, total: 4 }));
  };
  const hydrateRuntime = async () => {
    expect(pendingRuntimeReads.length).toBeGreaterThan(0);
    deferRuntime = false;
    await act(async () => pendingRuntimeReads.pop()!(authority));
  };
  const page = render(<MemoryRouter initialEntries={['/conversation']}>
    <I18nextProvider i18n={i18n}>
      <PreviewProvider persistNamespace='route-thinking-recovery' subscribeGlobalOpen={false}>
        <Link to='/settings'>Open settings</Link><Link to='/conversation'>Return to conversation</Link>
        <Routes>
          <Route path='/settings' element={<div>Settings page</div>} />
          <Route path='/conversation' element={<NomiChat conversation_id={conversationId} workspace=''
            modelSelection={selection} readOnly hideSendBox isProcessing={false} />} />
        </Routes>
      </PreviewProvider>
    </I18nextProvider>
  </MemoryRouter>);
  const thoughts = () => Array.from(page.container.querySelectorAll('[data-thinking-process-state]'));
  const headers = () => thoughts().map(thought => thought.querySelector<HTMLButtonElement>('[data-thinking-process-header]')!);
  const assertRecovered = () => {
    expect(thoughts().map(thought => thought.getAttribute('data-thinking-process-identity'))).toEqual([messageId(2), messageId(4)]);
    expect(thoughts().map(thought => thought.getAttribute('data-thinking-process-state'))).toEqual(['completed', 'running']);
    expect(headers().map(header => header.getAttribute('aria-expanded'))).toEqual(['false', 'true']);
    expect(thoughts()[1].getAttribute('data-thinking-body-length')).toBe('compact');
    expect(headers()[1].textContent).toContain('Thinking...');
    expect(page.container.querySelector('.turn-process-disclosure__label')?.textContent).not.toContain('--');
  };
  // The canonical page resolves first. The runtime GET is intentionally still
  // pending, so NomiChat supplies false processing with unknown hydration.
  await replyHistory(0);
  assertRecovered();
  await hydrateRuntime();
  assertRecovered();
  fireEvent.click(headers()[0]);
  expect(headers()[0].getAttribute('aria-expanded')).toBe('true');

  deferRuntime = true;
  fireEvent.click(page.getByRole('link', { name: 'Open settings' }));
  expect(page.getByText('Settings page')).toBeDefined();
  expect(thoughts()).toHaveLength(0);
  fireEvent.click(page.getByRole('link', { name: 'Return to conversation' }));
  await replyHistory(1);
  assertRecovered();
  await hydrateRuntime();

  await act(async () => stream!({ conversation_id: conversationId, turn_id: rootId,
    msg_id: messageId(4), type: 'thinking', data: { content: '', status: 'done' } }));
  await waitFor(() => expect(headers()[1].getAttribute('aria-expanded')).toBe('false'));
  expect(thoughts()[1].getAttribute('data-thinking-process-state')).toBe('completed');
  fireEvent.click(headers()[1]);
  expect(headers()[1].getAttribute('aria-expanded')).toBe('true');
  act(() => { for (const reconnect of reconnectListeners) reconnect(); });
  await replyHistory(2, 'done');
  expect(headers()[1].getAttribute('aria-expanded')).toBe('true');

  // Another HTTP snapshot begins before this stable step resumes after public
  // narration. Its older done result must not close the newer live phase.
  act(() => { for (const reconnect of reconnectListeners) reconnect(); });
  await act(async () => {
    stream!({ conversation_id: conversationId, turn_id: rootId, msg_id: messageId(5),
      type: 'text', data: { content: 'The file has been read.' } });
    stream!({ conversation_id: conversationId, turn_id: rootId, msg_id: messageId(4),
      type: 'thinking', data: { content: ' Verify the file.', status: 'thinking' } });
  });
  await waitFor(() => expect(thoughts()[1].getAttribute('data-thinking-process-state')).toBe('running'));
  await replyHistory(3, 'done');
  expect(thoughts()).toHaveLength(2);
  expect(thoughts()[1].getAttribute('data-thinking-process-state')).toBe('running');
  expect(headers()[1].getAttribute('aria-expanded')).toBe('true');
});
