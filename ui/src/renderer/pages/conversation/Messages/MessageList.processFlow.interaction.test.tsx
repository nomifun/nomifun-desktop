import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageToolCall, TMessage } from '@/common/chat/chatLib';
import type { MessageTimeline } from '@/common/protocolBindings/MessageTimeline';
import type { MessageTimelineSegment } from '@/common/protocolBindings/MessageTimelineSegment';
import { parseConversationId, parseKnowledgeBaseId, parseMessageId, parsePersistedArtifactId } from '@/common/types/ids';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { PreviewProvider } from '../Preview';
import MessageList from './MessageList';
import { MessageListProvider, MessageListLoadingProvider, useMessageLstCache, useUpdateMessageList, useAddOrUpdateMessage } from './hooks';
import { configService } from '@/common/config/configService';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';
import { ipcBridge } from '@/common';
import agentExecutionLocale from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import conversationLocale from '@/renderer/services/i18n/locales/en-US/conversation.json';
import { ThemeProvider } from '@/renderer/hooks/context/ThemeContext';
import idmmLocale from '@/renderer/services/i18n/locales/en-US/idmm.json';
import { useState } from 'react';
import { dispatchChatMessageJump } from '@/renderer/utils/chat/chatMinimapEvents';
import { emitter } from '@/renderer/utils/emitter';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { ExecutionProvider, useExecutionSafe } from '../execution/ExecutionContext';
import { executionId, leadConversation, leadConversationId, makeAttempt, makeDetail, makeStep, requestId } from '../../../../../test/fixtures/conversationDelegation';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages: messagesLocale, conversation: conversationLocale, agentExecution: agentExecutionLocale, idmm: idmmLocale } } } });
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000061');
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000062');
const messageId = (index: number) => parseMessageId(`0190f5fe-7c00-7a00-8000-${String(index).padStart(12, '0')}`);

let restoreListeners = () => {};
beforeEach(() => {
  const agentChanged = spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockReturnValue(() => {});
  const turnFiles = spyOn(ipcBridge.conversation.turnFiles, 'invoke').mockResolvedValue([]);
  const completed = spyOn(ipcBridge.conversation.turnCompleted, 'on').mockReturnValue(() => {});
  const reconnected = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const content = spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  const tree = spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  const entry = spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockImplementation(async ({ path }) => ({
    name: 'app.ts', path, size: 42, type: 'text/plain', lastModified: 0, isDirectory: false,
  }));
  const image = spyOn(ipcBridge.fs.getImageBase64, 'invoke').mockResolvedValue('data:image/png;base64,AAAA');
  const watchStart = spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(undefined);
  const watchStop = spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(undefined);
  const watchFiles = spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockImplementation(() => () => {});
  const files = spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
  restoreListeners = () => {
    agentChanged.mockRestore(); turnFiles.mockRestore(); completed.mockRestore(); reconnected.mockRestore();
    content.mockRestore(); tree.mockRestore(); entry.mockRestore(); metadata.mockRestore(); image.mockRestore();
    watchStart.mockRestore(); watchStop.mockRestore(); watchFiles.mockRestore(); files.mockRestore();
  };
});

afterEach(() => { cleanup(); restoreListeners(); });

test('permission recovery stays outside collapsed process, deduplicates retries, and follows exact accepted roots', async () => {
  const requestId = messageId(601);
  const nextRequestId = messageId(602);
  const nextTurnId = messageId(603);
  const permission = { schema: 'system-permission-required.v1' as const, capability: 'computer' as const,
    action_id: 'computer/input', operation: 'left_click', permissions: ['accessibility' as const], execution: 'not_executed' as const };
  const denied: IMessageToolCall = { id: 'permission-refusal', type: 'tool_call', conversation_id: conversationId,
    msg_id: messageId(604), turn_id: turnId, created_at: 2, content: { call_id: 'denied-call', name: 'computer_input',
      action_id: 'computer/input', args: { action: 'left_click' }, status: 'error', error_code: 'SYSTEM_PERMISSION_REQUIRED',
      result_seq: 10, permission_required: permission } };
  const initial: TMessage[] = [
    { id: 'permission-user', type: 'text', position: 'right', conversation_id: conversationId,
      msg_id: requestId, created_at: 1, content: { content: 'Please use the computer.' } }, denied,
    { id: 'permission-answer', type: 'text', position: 'left', conversation_id: conversationId, turn_id: turnId,
      msg_id: messageId(605), created_at: 3, content: { content: 'I need access for this action.' } },
    { id: 'permission-generic-blocked', type: 'tips', conversation_id: conversationId, turn_id: turnId,
      created_at: 3, content: { type: 'error', content: '', error: { message: 'The task has blocked work.',
        detail: 'Blocked criterion: Provide source artwork.',
        code: 'NOMIFUN_TASK_INCOMPLETE', taskIncompleteReason: 'blocked_work', retryable: false } } },
    { id: 'successful-blocked-criterion', type: 'tool_call', conversation_id: conversationId, turn_id: turnId, created_at: 3,
      content: { call_id: 'report_completion', name: 'report_completion', status: 'completed',
        args: { criteria: [{ id: 'source-artwork', status: 'blocked', reason: 'Need source artwork.' }] } } },
    { id: 'permission-finished', type: 'agent_status', conversation_id: conversationId, turn_id: turnId,
      created_at: 4, content: { backend: 'nomi', status: 'prepared', turn_summary: true,
        turn_state: 'failed', started_at_ms: 1, finished_at_ms: 4 } },
  ];
  const permissions = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue({ platform: 'macos', app_label: 'NomiFun',
    permissions: [{ kind: 'accessibility', state: 'denied', can_request: false, can_open_settings: true,
      requires_restart_after_grant: false, capabilities: ['computer_use'] }] });
  const Timeline = () => {
    const update = useUpdateMessageList();
    return <><MessageList />
      <button onClick={() => update(current => [...current, { ...denied, id: 'permission-retry', msg_id: messageId(606),
        content: { ...denied.content, call_id: 'denied-retry', result_seq: 11 } }])}>Repeat same request</button>
      <button onClick={() => update(current => [...current, { id: 'ordinary-next-request', type: 'text', position: 'right',
        conversation_id: conversationId, msg_id: nextRequestId, created_at: 5, content: { content: 'Let us just chat.' } }])}>Continue chatting</button>
      <button onClick={() => update(current => [...current, { ...denied, id: 'late-old-refusal', msg_id: messageId(607),
        created_at: 100, content: { ...denied.content, call_id: 'old-late', result_seq: 12 } }])}>Late old refusal</button>
      <button onClick={() => update(current => [...current, { ...denied, id: 'new-turn-refusal', msg_id: messageId(608),
        turn_id: nextTurnId, created_at: 6, content: { ...denied.content, call_id: 'new-real-call', result_seq: 20 } }])}>New actual refusal</button>
    </>;
  };
  try {
    const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
      <PreviewProvider persistNamespace='permission-turn-fixture' subscribeGlobalOpen={false}>
        <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', isProcessing: false, isTurnStateHydrated: true }}>
          <MessageListProvider initialValue={initial}><Timeline /></MessageListProvider>
        </ConversationProvider>
      </PreviewProvider>
    </I18nextProvider></MemoryRouter>);
    const notice = await page.findByTestId('system-permission-notice');
    expect(notice.closest('.turn-process-disclosure')).toBeNull();
    expect(page.getAllByTestId('system-permission-notice')).toHaveLength(1);
    const compact = page.container.querySelector('.message-error-note--compact');
    expect(compact).not.toBeNull();
    expect(compact!.textContent).toContain(conversationLocale.agentError.incompleteReasons.blocked_work.title);
    fireEvent.click(page.getByRole('button', { name: conversationLocale.agentError.expandDetails }));
    expect(page.getByText('Blocked criterion: Provide source artwork.')).toBeTruthy();
    fireEvent.click(page.getByRole('button', { name: messagesLocale.systemPermission.later }));
    fireEvent.click(page.getByText('Repeat same request'));
    expect(page.queryByTestId('system-permission-notice')).toBeNull();
    expect(permissions).toHaveBeenCalledTimes(1);
    fireEvent.click(page.getByText('Continue chatting'));
    fireEvent.click(page.getByText('Late old refusal'));
    expect(page.queryByTestId('system-permission-notice')).toBeNull();
    fireEvent.click(page.getByText('New actual refusal'));
    expect(await page.findByTestId('system-permission-notice')).toBeTruthy();
    await waitFor(() => expect(permissions).toHaveBeenCalledTimes(2));
  } finally { permissions.mockRestore(); }
});

test('active canonical Turn resolves historical request correlation and queued echo cannot steal its prompt', async () => {
  const requestId = messageId(701);
  const queuedRequestId = messageId(702);
  const initial: TMessage[] = [
    { id: 'history-source-request', type: 'text', position: 'right', conversation_id: conversationId,
      msg_id: requestId, created_at: 1, content: { content: 'Use the computer.' } },
    { id: 'history-permission-tool', type: 'tool_call', conversation_id: conversationId,
      msg_id: messageId(703), turn_id: requestId, created_at: 2,
      content: { call_id: 'history-call', name: 'computer_input', action_id: 'computer/input', status: 'error',
        error_code: 'SYSTEM_PERMISSION_REQUIRED', result_seq: 10, permission_required: {
          schema: 'system-permission-required.v1', capability: 'computer', action_id: 'computer/input',
          operation: 'left_click', permissions: ['accessibility'], execution: 'not_executed' } } },
    { id: 'queued-request-echo', type: 'text', position: 'right', status: 'pending', conversation_id: conversationId,
      msg_id: queuedRequestId, created_at: 3, content: { content: 'A queued follow-up.' } },
  ];
  const permissions = spyOn(ipcBridge.systemPermissions.get, 'invoke').mockResolvedValue({ platform: 'macos', app_label: 'NomiFun',
    permissions: [{ kind: 'accessibility', state: 'denied', can_request: false, can_open_settings: true,
      requires_restart_after_grant: false, capabilities: ['computer_use'] }] });
  try {
    const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
      <PreviewProvider persistNamespace='active-permission-history-fixture' subscribeGlobalOpen={false}>
        <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', isProcessing: true,
          activeTurnId: turnId, activeRequestMessageId: requestId, isTurnStateHydrated: true }}>
          <MessageListProvider initialValue={initial}><MessageList /></MessageListProvider>
        </ConversationProvider>
      </PreviewProvider>
    </I18nextProvider></MemoryRouter>);
    expect(await page.findByTestId('system-permission-notice')).toBeTruthy();
    expect(page.getAllByTestId('system-permission-notice')).toHaveLength(1);
    expect(page.getByTestId('system-permission-notice').closest('.turn-process-disclosure')).toBeNull();
    await waitFor(() => expect(permissions).toHaveBeenCalledTimes(1));
  } finally { permissions.mockRestore(); }
});

test('a restored automatic write-back notice sits under its answer and before the shared actions', async () => {
  const kb = parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000001');
  const load = spyOn(ipcBridge.knowledge.getBase, 'invoke').mockResolvedValue({ name: 'Python basics', knowledge_base_id: kb } as never);
  try {
    const initial: TMessage[] = [
      { id: 'knowledge-user', msg_id: turnId, type: 'text', position: 'right', conversation_id: conversationId,
        created_at: 1, content: { content: 'Explain Python loops.' } },
      { id: 'knowledge-answer', msg_id: messageId(201), turn_id: turnId, type: 'text', position: 'left',
        conversation_id: conversationId, created_at: 2, content: { content: 'The normal assistant answer.' } },
      { id: 'knowledge-terminal', msg_id: messageId(202), turn_id: turnId, type: 'agent_status', position: 'center',
        conversation_id: conversationId, created_at: 3, content: { backend: 'nomi', status: 'prepared',
          turn_summary: true, turn_state: 'completed', started_at_ms: 1000, finished_at_ms: 3000,
          turn_outcome: { kind: 'answered', message_ids: [messageId(201)] },
          knowledge_writeback: { status: 'written', written: [{ kb_id: kb, rel_path: 'notes/loops.md' }] } } },
    ];
    const page = render(<MemoryRouter><I18nextProvider i18n={i18n}><ThemeProvider>
      <PreviewProvider persistNamespace='knowledge-turn-fixture' subscribeGlobalOpen={false}>
        <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', isProcessing: false, isTurnStateHydrated: true }}>
          <MessageListProvider initialValue={initial}><MessageList emptySlot={<span>Welcome</span>} /></MessageListProvider>
        </ConversationProvider>
      </PreviewProvider>
    </ThemeProvider></I18nextProvider></MemoryRouter>);
    const notice = await page.findByTestId('knowledge-writeback-notice');
    expect(page.getAllByTestId('knowledge-writeback-notice')).toHaveLength(1);
    expect(notice.textContent).toContain('Knowledge saved · 1');
    const answer = await page.findByText('The normal assistant answer.');
    const actions = page.getByTestId('turn-actions');
    expect(page.getAllByTestId('message-copy-action')).toHaveLength(2);
    expect(answer.compareDocumentPosition(notice) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(notice.compareDocumentPosition(actions) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  } finally { load.mockRestore(); }
});

test('independent write-back stays under the completed answer while the next Turn continues', async () => {
  const kb = parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000001');
  const next = messageId(210);
  const summary = (status: 'running' | 'written', event_seq: number): TMessage => ({
    id: 'independent-summary', msg_id: messageId(202), turn_id: turnId, type: 'agent_status', position: 'center',
    conversation_id: conversationId, created_at: 3, content: { backend: 'nomi', status: 'prepared',
      turn_summary: true, turn_state: 'completed', started_at_ms: 1000, finished_at_ms: 3000,
      turn_outcome: { kind: 'answered', message_ids: [messageId(201)] },
      knowledge_writeback: { status, event_seq, written: status === 'written' ? [{ kb_id: kb, rel_path: 'notes/loops.md' }] : [] } },
  });
  function Updates() {
    const add = useAddOrUpdateMessage();
    return <>
      <button onClick={() => add(summary('written', 12))}>Confirm saved knowledge</button>
      <button onClick={() => add(summary('running', 11))}>Deliver late progress</button>
      <output data-testid='knowledge-current-busy'>{String(useConversationContextSafe()?.isProcessing)}</output>
    </>;
  }
  const initial: TMessage[] = [
    { id: 'old-user', msg_id: turnId, type: 'text', position: 'right', conversation_id: conversationId,
      created_at: 1, content: { content: 'First request' } },
    { id: 'old-answer', msg_id: messageId(201), turn_id: turnId, type: 'text', position: 'left',
      conversation_id: conversationId, status: 'finish', created_at: 2, content: { content: 'The first answer is complete.' } },
    summary('running', 11),
    { id: 'new-user', msg_id: next, type: 'text', position: 'right', conversation_id: conversationId,
      created_at: 4, content: { content: 'Continue with a second request' } },
    { id: 'new-thinking', msg_id: messageId(211), turn_id: next, type: 'thinking', position: 'left',
      conversation_id: conversationId, created_at: 5, content: { content: 'Work on the next answer', status: 'thinking' } },
  ];
  const load = spyOn(ipcBridge.knowledge.getBase, 'invoke').mockResolvedValue({ name: 'Python basics', knowledge_base_id: kb } as never);
  try {
    const page = render(<MemoryRouter><I18nextProvider i18n={i18n}><ThemeProvider>
      <PreviewProvider persistNamespace='independent-knowledge-turn' subscribeGlobalOpen={false}>
        <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', isProcessing: true,
          isTurnStateHydrated: true, activeTurnId: next, activeRequestMessageId: next, activeTurnStartedAt: 4000 }}>
          <MessageListProvider initialValue={initial}><Updates /><MessageList emptySlot={<span>Welcome</span>} /></MessageListProvider>
        </ConversationProvider>
      </PreviewProvider>
    </ThemeProvider></I18nextProvider></MemoryRouter>);
    const notice = await page.findByTestId('knowledge-writeback-notice');
    expect(notice.textContent).toContain('Saving knowledge');
    const answer = page.container.querySelector('#message-old-answer')!;
    expect(answer).toBeTruthy();
    await waitFor(() => expect(answer.querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain('The first answer is complete.'));
    const nextRequest = page.container.querySelector('#message-new-user')!;
    expect(answer.compareDocumentPosition(notice) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(notice.compareDocumentPosition(nextRequest) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(page.getByRole('button', { name: 'Confirm saved knowledge' }));
    await waitFor(() => expect(notice.textContent).toContain('Knowledge saved · 1'));
    fireEvent.click(page.getByRole('button', { name: 'Deliver late progress' }));
    await waitFor(() => expect(page.getByTestId('knowledge-current-busy').textContent).toBe('true'));
    expect(page.getAllByTestId('knowledge-writeback-notice')).toHaveLength(1);
    expect(notice.textContent).toContain('Knowledge saved · 1');
  } finally { load.mockRestore(); }
});

test.each([
  { errorAt: undefined, cleanupProven: true, pausedAt: 10, expected: 1, projected: true },
  { errorAt: 11, cleanupProven: true, pausedAt: 10, expected: 1, projected: false },
  { errorAt: 9, cleanupProven: true, pausedAt: 10, expected: 1, projected: false },
  { errorAt: 11, cleanupProven: false, pausedAt: 10, expected: 1, projected: false },
  { errorAt: 11, cleanupProven: true, pausedAt: undefined, expected: 1, projected: false },
  { errorAt: 11, cleanupProven: false, pausedAt: 10, foreignTurn: true, expected: 2, projected: true },
])('pause uses the shared error note without duplicating the current diagnosis %j', (scenario) => {
  const initial: TMessage[] = scenario.errorAt === undefined ? [] : [{
    id: 'canonical-error', msg_id: messageId(4), type: 'tips', conversation_id: conversationId,
    turn_id: 'foreignTurn' in scenario && scenario.foreignTurn ? messageId(5) : turnId,
    created_at: scenario.errorAt, content: { type: 'error', content: '',
      error: { message: 'Account action required', code: 'USER_LLM_PROVIDER_BILLING_REQUIRED', retryable: false } },
  }];
  const pause = { turnId, reason: 'EXECUTION_USER_REQUESTED', cleanupProven: scenario.cleanupProven,
    pausedAt: scenario.pausedAt, error: { message: '', agentLabel: 'Admitted Agent', modelName: 'admitted-model' } };
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}><ThemeProvider>
    <PreviewProvider persistNamespace='shared-pause-note-test' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', executionPause: pause }}>
        <MessageListProvider initialValue={initial}><MessageList emptySlot={<span>Welcome</span>} /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </ThemeProvider></I18nextProvider></MemoryRouter>);
  expect(page.container.querySelectorAll('.message-error-note').length).toBe(scenario.expected);
  expect(page.queryByTestId('conversation-pause-error') !== null).toBe(scenario.projected);
  expect(page.container.querySelector('[data-testid="execution-pause-notice"]')).toBeNull();
  expect(page.queryByText('Welcome')).toBeNull();
  expect(page.queryByRole('button', { name: 'Retry' })).toBeNull();
  if (!scenario.cleanupProven && scenario.projected) expect(page.container.textContent).toContain(conversationLocale.agentError.codes.EXECUTION_CLEANUP_UNCONFIRMED.title);
  expect(page.queryByRole('button', { name: 'End this turn' })).toBeNull();
});

test('decision question jumps load an older history page and historical failures remain standalone notes', async () => {
  const questionId = messageId(91);
  const question: TMessage = { id: 'question', type: 'text', position: 'left', conversation_id: conversationId,
    message_id: questionId, msg_id: questionId, created_at: 1, content: { content: 'Choose a cache strategy?' } };
  const notice: TMessage = { id: 'notice', type: 'tips', position: 'center', conversation_id: conversationId,
    message_id: messageId(92), msg_id: messageId(92), created_at: 2, content: { content: '', type: 'error', idmm_notice: {
      status: 'failed', created_at: 2, decision: { intervention_id: messageId(93), source: 'rule',
        reason_code: 'rule_cannot_answer', rationale: 'This question needs human input.',
        question: { message_id: questionId, sequence: 1, fingerprint: 'a'.repeat(64) } },
    } } };
  let olderLoads = 0;
  const PagingTimeline = () => {
    const update = useUpdateMessageList();
    const [loaded, setLoaded] = useState(false);
    return <MessageList hasMoreOlder={!loaded} loadingOlder={false} onLoadOlder={async () => {
      olderLoads++;
      update(current => [question, ...current]);
      setLoaded(true);
    }} />;
  };
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='idmm-question-history-test' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true, isProcessing: false }}>
        <MessageListProvider initialValue={[notice]}><PagingTimeline /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>);
  const renderedNotice = page.getByTestId('idmm-decision-notice');
  expect(renderedNotice.closest('.turn-process-disclosure')).toBeNull();
  expect(page.queryByTestId('conversation-current-activity')).toBeNull();
  act(() => dispatchChatMessageJump({ conversation_id: conversationId, messageId: questionId, loadOlder: true }));
  await waitFor(() => expect(olderLoads).toBe(1));
  await waitFor(() => expect(page.container.querySelector(`[data-message-business-id="${questionId}"]`)).not.toBeNull());
  expect(page.getByTestId('idmm-decision-notice').closest('.turn-process-disclosure')).toBeNull();
});

test('a failed question history page stops automatic loading and a second click can retry successfully', async () => {
  const questionId = messageId(94);
  const question: TMessage = { id: 'older-question', type: 'text', position: 'left', conversation_id: conversationId,
    message_id: questionId, msg_id: questionId, created_at: 1, content: { content: 'Choose a cache strategy?' } };
  const reply: TMessage = { id: 'automatic-reply', type: 'text', position: 'right', conversation_id: conversationId,
    message_id: messageId(95), msg_id: messageId(95), created_at: 2, content: { content: '2', idmm_decision: {
      intervention_id: messageId(96), source: 'rule', reason_code: 'rule_selected_recommended_option',
      rationale: 'Choose the recommended safe option.', question: { message_id: questionId, sequence: 1, fingerprint: 'a'.repeat(64) },
    } } };
  let olderRequests = 0;
  const errors = spyOn(console, 'error').mockImplementation(() => {});
  const completed = spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(() => () => {});
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => () => {});
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementation(async query => {
    if (!query.cursor) return { items: [reply], has_more: true, total: 2, next_cursor: `${reply.created_at}:${reply.message_id}` };
    olderRequests++;
    if (olderRequests === 1) throw new Error('fixture page unavailable');
    return { items: [question], has_more: false, total: 2 };
  });
  const Timeline = () => {
    const paging = useMessageLstCache(conversationId);
    return <MessageList onLoadOlder={paging.loadOlder} hasMoreOlder={paging.hasMore} loadingOlder={paging.loadingOlder} />;
  };
  try {
    const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
      <PreviewProvider persistNamespace='idmm-question-failed-history-test' subscribeGlobalOpen={false}>
        <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true, isProcessing: false }}>
          <MessageListProvider><MessageListLoadingProvider><Timeline /></MessageListLoadingProvider></MessageListProvider>
        </ConversationProvider>
      </PreviewProvider>
    </I18nextProvider></MemoryRouter>);
    await waitFor(() => expect(page.getByRole('button', { name: 'View original question' })).toBeTruthy());
    fireEvent.click(page.getByRole('button', { name: 'View original question' }));
    await waitFor(() => expect(olderRequests).toBe(1));
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 80)); });
    expect(olderRequests).toBe(1);
    expect(page.container.querySelector(`[data-message-business-id="${questionId}"]`)).toBeNull();
    fireEvent.click(page.getByRole('button', { name: 'View original question' }));
    await waitFor(() => expect(olderRequests).toBe(2));
    await waitFor(() => expect(page.container.querySelector(`[data-message-business-id="${questionId}"]`)).not.toBeNull());
  } finally { history.mockRestore(); errors.mockRestore(); completed.mockRestore(); reconnect.mockRestore(); }
});

test('a live journal tracks thinking lifecycle and preserves completed thought disclosures', () => {
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const messages: TMessage[] = [
    { ...base, id: 'user', message_id: turnId, msg_id: turnId, type: 'text', created_at: 1, position: 'right', content: { content: 'Inspect the files' } },
    { ...base, id: 'thought-1', msg_id: messageId(2), type: 'thinking', created_at: 2, content: { content: 'First reasoning is complete.', status: 'done' } },
    { ...base, id: 'progress', msg_id: messageId(3), type: 'text', created_at: 3, content: { content: 'I will inspect the source now.' } },
    { ...base, id: 'read-call', msg_id: messageId(4), type: 'tool_call', created_at: 4, content: { call_id: 'read-current', name: 'read_file', status: 'completed', args: { path: 'src/app.ts' }, output: 'Source contents', artifacts: [] } },
    { ...base, id: 'thought-2', msg_id: messageId(5), type: 'thinking', created_at: 5, content: { content: 'Second reasoning is in progress.', status: 'thinking' } },
  ];
  const FinishCurrentThought = () => {
    const updateMessages = useUpdateMessageList();
    return <button onClick={() => updateMessages(current => current.map(message =>
      message.id === 'thought-2' && message.type === 'thinking'
        ? { ...message, content: { ...message.content, status: 'done' } }
        : message
    ))}>Finish current thought</button>;
  };
  const view = (running: boolean) => <MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='live-thought-status-test' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true, isProcessing: running, activeTurnId: turnId }}>
        <MessageListProvider initialValue={messages}><MessageList /><FinishCurrentThought /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>;
  const page = render(view(true));

  expect(page.queryByTestId('conversation-current-activity')).toBeNull();

  const thoughts = page.container.querySelectorAll('[data-thinking-process-state]');
  expect(thoughts).toHaveLength(2);
  expect(Array.from(thoughts, thought => thought.getAttribute('data-thinking-process-state'))).toEqual(['completed', 'running']);
  expect(thoughts[0].querySelector('.arco-spin')).toBeNull();
  expect(thoughts[1].querySelector('.arco-spin')).not.toBeNull();
  expect(page.container.querySelectorAll('[data-thinking-process-state="running"]')).toHaveLength(1);
  const firstHeader = thoughts[0].querySelector<HTMLButtonElement>('[data-thinking-process-header]')!;
  const secondHeader = thoughts[1].querySelector<HTMLButtonElement>('[data-thinking-process-header]')!;
  expect(firstHeader.textContent).toContain('Thought complete');
  expect(secondHeader.textContent).toContain('Thinking...');
  expect(thoughts[0].querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain('First reasoning is complete.');
  expect(firstHeader.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(firstHeader);
  expect(firstHeader.getAttribute('aria-expanded')).toBe('true');
  fireEvent.click(firstHeader);
  expect(firstHeader.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(firstHeader);
  expect(firstHeader.getAttribute('aria-expanded')).toBe('true');
  expect(thoughts[0].querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain('First reasoning is complete.');
  fireEvent.click(secondHeader);
  expect(secondHeader.getAttribute('aria-expanded')).toBe('false');

  fireEvent.click(page.getByRole('button', { name: 'Finish current thought' }));
  const completedThoughts = page.container.querySelectorAll('[data-thinking-process-state]');
  expect(Array.from(completedThoughts, thought => thought.getAttribute('data-thinking-process-state'))).toEqual(['completed', 'completed']);
  expect(page.container.querySelector('[data-thinking-process-state="running"]')).toBeNull();
  expect(completedThoughts[1].querySelector('.arco-spin')).toBeNull();
  const completedHeader = completedThoughts[1].querySelector<HTMLButtonElement>('[data-thinking-process-header]')!;
  expect(completedHeader.textContent).toContain('Thought complete');
  expect(completedHeader.getAttribute('aria-expanded')).toBe('false');
  expect(page.queryByTestId('conversation-current-activity')).toBeNull();
  fireEvent.click(completedHeader);
  expect(completedHeader.getAttribute('aria-expanded')).toBe('true');
  expect(completedThoughts[1].querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain('Second reasoning is in progress.');
  expect(messages[4].type === 'thinking' && messages[4].content.status).toBe('thinking');

  fireEvent.click(page.getByRole('button', { name: messagesLocale.turnProcess.collapse }));
  expect(page.container.querySelector('.turn-process-disclosure__body')).toBeNull();
  expect(page.queryByTestId('conversation-current-activity')).toBeNull();
  page.rerender(view(false));
  expect(page.queryByTestId('conversation-current-activity')).toBeNull();
});

test('compact thinking resumes in its stable row after narration and a delayed old Turn cannot stop the clock', async () => {
  const config = spyOn(configService, 'get').mockImplementation(((key: string) =>
    key === 'chat.thinking.contentLength' ? 'compact' : undefined) as typeof configService.get);
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const thought: TMessage = { ...base, id: 'step-thought', msg_id: messageId(2), type: 'thinking',
    created_at: Date.now() - 1000, content: { content: 'Inspect. ', status: 'thinking' } };
  const messages: TMessage[] = [
    { ...base, id: 'user', msg_id: turnId, type: 'text', position: 'right',
      created_at: Date.now() - 2000, content: { content: 'Inspect the files' } },
    thought,
  ];
  const StreamControls = () => {
    const add = useAddOrUpdateMessage();
    return <>
      <button onClick={() => {
        add({ ...thought, id: 'thought-done', content: { content: '', status: 'done' } });
        add({ ...base, id: 'narration', msg_id: messageId(3), type: 'text', content: { content: 'Reading the source.' } });
      }}>Narrate</button>
      <button onClick={() => add({ ...thought, id: 'thought-resumed',
        content: { content: 'Inspect. ', status: 'thinking' } })}>Resume reasoning</button>
      <button onClick={() => add({ ...base, id: 'delayed-old', msg_id: messageId(7), turn_id: messageId(8),
        type: 'thinking', content: { content: 'Prior task.', status: 'done' } })}>Deliver older event</button>
    </>;
  };
  try {
    const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
      <PreviewProvider persistNamespace='compact-resumed-thought-test' subscribeGlobalOpen={false}>
        <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true,
          isProcessing: true, activeTurnId: turnId, activeRequestMessageId: turnId }}>
          <MessageListProvider initialValue={messages}><MessageList /><StreamControls /></MessageListProvider>
        </ConversationProvider>
      </PreviewProvider>
    </I18nextProvider></MemoryRouter>);
    const thinking = () => page.container.querySelector('[data-thinking-process-state]')!;
    expect(thinking().getAttribute('data-thinking-body-length')).toBe('compact');
    expect(thinking().getAttribute('data-thinking-process-state')).toBe('running');
    fireEvent.click(page.getByRole('button', { name: 'Narrate' }));
    await waitFor(() => expect(thinking().getAttribute('data-thinking-process-state')).toBe('completed'));
    fireEvent.click(page.getByRole('button', { name: 'Resume reasoning' }));
    await waitFor(() => expect(thinking().getAttribute('data-thinking-process-state')).toBe('running'));
    expect(page.container.querySelectorAll('[data-thinking-process-state]')).toHaveLength(1);
    expect(thinking().getAttribute('data-thinking-process-state')).toBe('running');
    expect(thinking().querySelector('[data-thinking-process-header]')?.textContent).toContain('Thinking...');
    expect(thinking().querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain('Inspect. Inspect.');
    fireEvent.click(page.getByRole('button', { name: 'Deliver older event' }));
    await waitFor(() => expect(page.container.querySelectorAll('.turn-process-disclosure')).toHaveLength(2));
    expect(page.container.querySelector('.turn-process-disclosure--live')).not.toBeNull();
    expect(thinking().getAttribute('data-thinking-process-state')).toBe('running');
  } finally { config.mockRestore(); }
});

test('canonical terminal metadata closes stale thinking while the session still reports processing', () => {
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const messages: TMessage[] = [
    { ...base, id: 'user', message_id: turnId, msg_id: turnId, type: 'text', created_at: 1, position: 'right', content: { content: 'Inspect the files' } },
    { ...base, id: 'stale-thought', msg_id: messageId(2), type: 'thinking', created_at: 2, content: { content: 'The source inspection reasoning.', status: 'thinking' } },
    { ...base, id: 'final', msg_id: messageId(3), type: 'text', created_at: 3, content: { content: 'The inspection is complete.' } },
    { ...base, id: 'terminal-metadata', msg_id: messageId(4), type: 'agent_status', created_at: 4,
      content: { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed', started_at_ms: 1, finished_at_ms: 3,
        turn_outcome: { kind: 'answered', message_ids: [messageId(3)] } } },
  ];
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='terminal-thought-status-test' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true, isProcessing: true, activeTurnId: turnId }}>
        <MessageListProvider initialValue={messages}><MessageList /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>);

  expect(page.container.querySelector('.turn-process-disclosure--live')).toBeNull();
  expect(page.queryByTestId('conversation-current-activity')).toBeNull();
  expect(page.container.querySelector('.turn-process-disclosure__body')).toBeNull();
  expect(Array.from(page.container.querySelectorAll('.markdown-shadow'), node => node.shadowRoot?.textContent ?? '')
    .some(text => text.includes('The inspection is complete.'))).toBe(true);
  fireEvent.click(page.getByRole('button', { name: messagesLocale.turnProcess.expand }));
  const thought = page.container.querySelector('[data-thinking-process-state]');
  expect(thought?.getAttribute('data-thinking-process-state')).toBe('completed');
  expect(thought?.querySelector('.arco-spin')).toBeNull();
  expect(thought?.querySelector('[data-thinking-process-header]')?.textContent).toContain('Thought complete');
  expect(thought?.querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain('The source inspection reasoning.');
  expect(messages[1].type === 'thinking' && messages[1].content.status).toBe('thinking');
});

test('waiting for the first response and running a tool call do not add a duplicate activity footer', () => {
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const messages: TMessage[] = [
    { ...base, id: 'user', message_id: turnId, msg_id: turnId, type: 'text', created_at: 1, position: 'right', content: { content: 'Inspect the files' } },
  ];
  const StartToolCall = () => {
    const updateMessages = useUpdateMessageList();
    return <button onClick={() => updateMessages(current => [...current,
      { ...base, id: 'thought', msg_id: messageId(2), type: 'thinking', created_at: 2, content: { content: 'The file to inspect is selected.', status: 'done' } },
      { ...base, id: 'read-call', msg_id: messageId(3), type: 'tool_call', created_at: 3, content: { call_id: 'read-active', name: 'read_file', status: 'running', args: { path: 'src/app.ts' }, artifacts: [] } },
    ])}>Start tool call</button>;
  };
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='waiting-activity-test' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true, isProcessing: true, activeTurnId: turnId }}>
        <MessageListProvider initialValue={messages}><MessageList /><StartToolCall /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>);

  expect(page.queryByTestId('conversation-current-activity')).toBeNull();

  fireEvent.click(page.getByRole('button', { name: 'Start tool call' }));
  const thought = page.container.querySelector('[data-thinking-process-state]');
  expect(thought?.getAttribute('data-thinking-process-state')).toBe('completed');
  expect(thought?.querySelector('.arco-spin')).toBeNull();
  expect(thought?.querySelector('[data-thinking-process-header]')?.textContent).toContain('Thought complete');
  expect(page.container.querySelector('[data-tool-call-id="read-active"]')).not.toBeNull();
  expect(page.container.querySelector('[data-tool-call-id="read-active"] .turn-process-trace__row--running')).not.toBeNull();
  expect(page.queryByTestId('conversation-current-activity')).toBeNull();
  fireEvent.click(page.getByRole('button', { name: messagesLocale.turnProcess.collapse }));
  expect(page.container.querySelector('.turn-process-disclosure__body')).toBeNull();
  expect(page.queryByTestId('conversation-current-activity')).toBeNull();
});

test('a completed journal defaults closed and expands its full reasoning and calls without hiding the final reply', () => {
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const firstCall: IMessageToolCall = { ...base, id: 'first-call', message_id: messageId(3), msg_id: messageId(3), created_at: 3,
    type: 'tool_call', content: { call_id: 'read-1', name: 'read_file', status: 'error', args: { path: 'src/first.ts' },
      output: 'First attempt failed', artifacts: [] } };
  const retry: IMessageToolCall = { ...firstCall, id: 'retry-call', message_id: messageId(6), msg_id: messageId(6), created_at: 6,
    content: { ...firstCall.content, call_id: 'read-2', status: 'completed', output: 'Second attempt result' } };
  const messages: TMessage[] = [
    { ...base, id: 'user', message_id: turnId, msg_id: turnId, type: 'text', created_at: 1, position: 'right', content: { content: 'Inspect the files' } },
    { ...base, id: 'thought-1', msg_id: messageId(2), type: 'thinking', created_at: 2, content: { content: '**First reasoning**\n\nInspect the source.', status: 'done' } },
    firstCall,
    { ...base, id: 'progress', msg_id: messageId(4), type: 'text', created_at: 4, content: { content: 'I will retry with the available source.' } },
    { ...base, id: 'thought-2', msg_id: messageId(5), type: 'thinking', created_at: 5, content: { content: 'Second reasoning', status: 'done' } },
    retry,
    { ...base, id: 'final', msg_id: messageId(7), type: 'text', created_at: 7, content: { content: 'The inspection is complete.' } },
    { ...base, id: 'terminal', msg_id: messageId(8), type: 'agent_status', created_at: 8, content: { backend: 'nomi',
      status: 'prepared', turn_summary: true, turn_state: 'completed', turn_outcome: { kind: 'answered', message_ids: [messageId(7)] } } },
  ];
  const view = (running: boolean) => <MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='process-flow-test' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true, isProcessing: running }}>
        <MessageListProvider initialValue={messages}><MessageList /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>;

  const page = render(view(true));
  expect(page.container.querySelector('.turn-process-disclosure__body')).not.toBeNull();
  const thoughtHeaders = page.container.querySelectorAll('[data-thinking-process-header]');
  expect(thoughtHeaders).toHaveLength(2);
  thoughtHeaders.forEach((header) => expect(header.getAttribute('aria-expanded')).toBe('false'));
  page.rerender(view(false));
  expect(page.container.querySelector('.turn-process-disclosure__body')).toBeNull();
  expect(Array.from(page.container.querySelectorAll('.markdown-shadow'), (node) => node.shadowRoot?.textContent ?? '')
    .filter((text) => text.includes('The inspection is complete.'))).toHaveLength(1);
  fireEvent.click(page.getByRole('button', { name: messagesLocale.turnProcess.expand }));
  const steps = Array.from(page.container.querySelectorAll('.turn-process-disclosure__item'));
  expect(steps.map((step) => step.className.match(/turn-process-disclosure__item--(thinking|tool|text)/)?.[1]))
    .toEqual(['thinking', 'tool', 'text', 'thinking', 'tool']);
  expect(steps[0].querySelector('.markdown-shadow')?.shadowRoot?.querySelector('strong')?.textContent).toBe('First reasoning');
  expect(steps[3].querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain('Second reasoning');
  expect(page.container.querySelectorAll('.turn-process-receipt')).toHaveLength(0);
  const calls = page.container.querySelectorAll('[data-tool-call-id]');
  expect(Array.from(calls, (call) => call.getAttribute('data-tool-call-id'))).toEqual(['read-1', 'read-2']);
  calls.forEach((call) => fireEvent.click(call.querySelector('button')!));
  expect(calls[0].textContent).toContain('First attempt failed');
  expect(calls[1].textContent).toContain('Second attempt result');
  expect(Array.from(page.container.querySelectorAll('.markdown-shadow'), (node) => node.shadowRoot?.textContent ?? '')
    .filter((text) => text.includes('The inspection is complete.'))).toHaveLength(1);
  expect(firstCall.content.status).toBe('error');
  expect(messages[1].type).toBe('thinking');
});

test('finished root reply keeps the collaboration clock live, opens tasks, and collapses only after all work finishes', async () => {
  const step = makeStep(1, { title: 'Build flight game' });
  const waiting = makeStep(2, { title: 'Verify gameplay', status: 'pending' });
  const attempt = makeAttempt(step);
  const start = Date.now() - 3000;
  const detail = makeDetail({ steps: [step, waiting], attempts: [attempt] });
  Object.assign(detail.execution, { created_at: start + 1000, updated_at: start + 2000 });
  const get = spyOn(ipcBridge.agentExecution.get, 'invoke').mockResolvedValue(detail);
  const changed = spyOn(ipcBridge.agentExecution.events.changed, 'on').mockImplementation(() => () => {});
  const thinking = spyOn(ipcBridge.agentExecution.events.leadThinking, 'on').mockImplementation(() => () => {});
  const reconnect = spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(() => () => {});
  const Probe = () => <>
    <output data-testid='root-processing'>{String(useConversationContextSafe()?.isProcessing)}</output>
    <output data-testid='projected-step'>{useExecutionSafe()?.projectedStepId}</output>
  </>;
  const messages: TMessage[] = [
    { id: 'delegation-request', conversation_id: leadConversationId, msg_id: requestId, message_id: requestId,
      position: 'right', type: 'text', created_at: start, content: { content: 'Build a flight game with subagents', display_at_ms: start } },
    { id: 'delegation-thought', conversation_id: leadConversationId, turn_id: requestId, msg_id: messageId(80),
      position: 'left', type: 'thinking', created_at: start + 200, content: { content: 'I will delegate implementation and verification.', status: 'done' } },
    { id: 'delegation-answer', conversation_id: leadConversationId, turn_id: requestId, msg_id: messageId(81),
      position: 'left', type: 'text', created_at: start + 2000, content: { content: 'The subagent tasks have started.' } },
  ];
  try {
    const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
      <PreviewProvider persistNamespace='delegation-progress-test' subscribeGlobalOpen={false}>
        <ExecutionProvider conversation={leadConversation}>
          <ConversationProvider value={{ conversation_id: leadConversationId, type: 'nomi', readOnly: true, isProcessing: false }}>
            <MessageListProvider initialValue={messages}><MessageList /><Probe /></MessageListProvider>
          </ConversationProvider>
        </ExecutionProvider>
      </PreviewProvider>
    </I18nextProvider></MemoryRouter>);
    await waitFor(() => expect(page.getByTestId('conversation-delegation').textContent).toContain('Build flight game'));
    expect(page.container.querySelector('.turn-process-disclosure--live')).not.toBeNull();
    expect(page.getByTestId('root-processing').textContent).toBe('false');
    expect(page.container.querySelectorAll('[data-thinking-process-header]')).toHaveLength(1);
    expect(page.container.querySelector('[data-thinking-process-header]')?.textContent).not.toMatch(/\d+s/);
    expect(page.container.querySelector('.turn-process-disclosure__label')?.textContent).toContain('3s');
    await waitFor(() => expect(page.container.querySelector('.turn-process-disclosure__label')?.textContent).toContain('4s'), { timeout: 1600 });
    fireEvent.click(page.getByRole('button', { name: 'View subtask: Build flight game' }));
    expect(page.getByTestId('projected-step').textContent).toBe(step.step_id);
    expect(page.getByTestId('root-processing').textContent).toBe('false');
    expect(Array.from(page.container.querySelectorAll('.markdown-shadow'), node => node.shadowRoot?.textContent ?? '')
      .some(text => text.includes('The subagent tasks have started.'))).toBe(true);

    get.mockResolvedValue(makeDetail({ execution: { ...detail.execution, status: 'completed', updated_at: start + 4000, event_sequence: 2 },
      steps: detail.steps.map(value => ({ ...value, status: 'completed' })), attempts: [{ ...attempt, status: 'completed', finished_at: start + 4000 }] }));
    act(() => changed.mock.calls.forEach(([listener]) => listener({ execution_id: executionId, sequence: 2, change_kind: 'status_changed' })));
    await waitFor(() => expect(Boolean(page.container.querySelector('.turn-process-disclosure--live'))).toBe(false));
    await waitFor(() => expect(Boolean(page.container.querySelector('.turn-process-disclosure__body'))).toBe(false));
    expect(page.container.querySelector('.turn-process-disclosure__label')?.textContent).toContain('4s');
    expect(page.container.querySelector('.turn-process-disclosure__activity')?.textContent).toContain('Completed');
    fireEvent.click(page.getByRole('button', { name: messagesLocale.turnProcess.expand }));
    expect(page.getByTestId('conversation-delegation').textContent).toContain(i18n.t('agentExecution.progress.summary', { done: 2, total: 2 }));
    expect(page.getByTestId('root-processing').textContent).toBe('false');
  } finally { cleanup(); get.mockRestore(); changed.mockRestore(); thinking.mockRestore(); reconnect.mockRestore(); }
});

test.each(['completed', 'failed', 'canceled'] as const)('canonical file list survives %s outcome and partial message history without changing its placement', async (outcome) => {
  const query = spyOn(ipcBridge.conversation.turnFiles, 'invoke').mockResolvedValue([{ turn_id: turnId, complete: true, files: [
    { identity: 'workspace:script', relative_path: 'build_docx.py', file_name: 'build_docx.py', path: '/workspace/build_docx.py', operation: 'create', size_bytes: 42 },
    { identity: 'workspace:document', relative_path: 'report.docx', file_name: 'report.docx', path: '/workspace/report.docx', operation: 'create', size_bytes: 128 },
    { identity: 'workspace:deleted', relative_path: 'old.txt', file_name: 'old.txt', path: '/workspace/old.txt', operation: 'delete' },
  ] }]);
  // Tool/process messages are deliberately outside the loaded history window.
  const initial: TMessage[] = [
    { id: 'file-request', type: 'text', position: 'right', conversation_id: conversationId, message_id: turnId, msg_id: turnId, created_at: 100,
      content: { content: 'Write a document' } },
    { id: 'file-answer', type: 'text', position: 'left', conversation_id: conversationId, message_id: messageId(500), msg_id: messageId(500), turn_id: turnId,
      created_at: 200, content: { content: 'The files are ready.' } },
  ];
  if (outcome === 'failed') initial.push({ id: 'terminal-file-error', type: 'tips', conversation_id: conversationId,
    message_id: messageId(501), msg_id: messageId(501), turn_id: turnId, created_at: 300,
    content: { type: 'error', content: 'Execution failed', finished_at_ms: 300,
      turn_outcome: { kind: 'failed', message_ids: [messageId(500)] } } });
  if (outcome === 'canceled') initial.push({ id: 'terminal-file-canceled', type: 'agent_status', conversation_id: conversationId,
    message_id: messageId(501), msg_id: messageId(501), turn_id: turnId, created_at: 300,
    content: { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'cancelled', started_at_ms: 100, finished_at_ms: 300,
      turn_outcome: { kind: 'cancelled', message_ids: [messageId(500)] } } });
  if (outcome === 'completed') initial.push({ id: 'file-completion', msg_id: messageId(502), turn_id: turnId,
    conversation_id: conversationId, type: 'agent_status', created_at: 300, content: { backend: 'nomi', status: 'prepared',
      turn_summary: true, turn_outcome: { kind: 'answered', message_ids: [messageId(500)] } } });
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}><ThemeProvider>
    <PreviewProvider persistNamespace='canonical-turn-files-test' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, workspace: '/workspace', type: 'nomi', readOnly: true,
        isProcessing: false, isTurnStateHydrated: true }}>
        <MessageListProvider initialValue={initial}><MessageList hasMoreOlder /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </ThemeProvider></I18nextProvider></MemoryRouter>);
  await waitFor(() => expect(page.getByText('This turn involves 3 files')).toBeTruthy());
  const card = page.getByTestId('turn-deliverables-card');
  expect(card.getAttribute('data-partial')).toBeNull();
  expect(card.querySelectorAll('[data-deliverable-path]')).toHaveLength(3);
  expect(page.getByText('report.docx')).toBeTruthy();
  expect(page.getByText('Deleted')).toBeTruthy();
  expect(query).toHaveBeenCalledWith({ conversation_id: conversationId, turn_ids: [turnId] });
  const reply = page.getByText('The files are ready.');
  expect(reply.compareDocumentPosition(card) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  const actions = page.getByTestId('turn-actions');
  expect(card.compareDocumentPosition(actions) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

test('a closed Turn distinguishes unknown empty file coverage from pending and confirmed empty queries', async () => {
  let resolveInitial!: (value: Awaited<ReturnType<typeof ipcBridge.conversation.turnFiles.invoke>>) => void;
  const query = spyOn(ipcBridge.conversation.turnFiles, 'invoke')
    .mockImplementationOnce(() => new Promise(resolve => { resolveInitial = resolve; }))
    .mockResolvedValue([{ turn_id: turnId, complete: false, files: [] }]);
  const initial: TMessage[] = [
    { id: 'empty-file-request', type: 'text', position: 'right', conversation_id: conversationId, message_id: turnId, msg_id: turnId,
      created_at: 100, content: { content: 'Check the workspace' } },
    { id: 'empty-file-answer', type: 'text', position: 'left', conversation_id: conversationId, message_id: messageId(510), msg_id: messageId(510),
      turn_id: turnId, created_at: 200, content: { content: 'The task has ended.' } },
    { id: 'empty-file-completion', msg_id: messageId(511), turn_id: turnId, conversation_id: conversationId,
      type: 'agent_status', created_at: 300, content: { backend: 'nomi', status: 'prepared', turn_summary: true,
        turn_outcome: { kind: 'answered', message_ids: [messageId(510)] } } },
  ];
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='empty-canonical-turn-files-test' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, workspace: '/workspace', type: 'nomi', readOnly: true,
        isProcessing: false, isTurnStateHydrated: true }}>
        <MessageListProvider initialValue={initial}><MessageList /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>);
  expect(page.queryByTestId('turn-deliverables-card')).toBeNull();
  await act(async () => { resolveInitial([{ turn_id: turnId, complete: true, files: [] }]); });
  expect(page.queryByTestId('turn-deliverables-card')).toBeNull();
  act(() => { emitter.emit('conversation.turn.settled', conversationId); });
  await waitFor(() => expect(page.getByText('This turn involves 0 files (list incomplete)')).toBeTruthy());
  const card = page.getByTestId('turn-deliverables-card');
  expect(card.getAttribute('data-partial')).toBe('true');
  expect(card.querySelectorAll('[data-deliverable-path]')).toHaveLength(0);
  expect(query).toHaveBeenCalledTimes(2);
  expect(card.compareDocumentPosition(page.getByTestId('turn-actions')) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

test('paused and resumed Turns hide file headings and image gallery until canonical finish overrides stale pause', async () => {
  const query = spyOn(ipcBridge.conversation.turnFiles, 'invoke').mockResolvedValue([{ turn_id: turnId, complete: true, files: [
    { identity: 'workspace:report', relative_path: 'report.docx', file_name: 'report.docx', path: '/workspace/report.docx', operation: 'create', size_bytes: 22 },
  ] }]);
  const initial: TMessage[] = [
    { id: 'pause-file-request', type: 'text', position: 'right', conversation_id: conversationId, message_id: turnId, msg_id: turnId,
      created_at: 100, content: { content: 'Generate an image and document' } },
    { id: 'pause-file-image', type: 'tool_call', conversation_id: conversationId, message_id: messageId(520), msg_id: messageId(520), turn_id: turnId,
      created_at: 150, content: { call_id: 'image-call', name: 'image', status: 'completed', artifacts: [
        { id: parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000521'), kind: 'image', mime_type: 'image/png', path: '/workspace/generated.png',
          relative_path: 'generated.png', size_bytes: 100, sha256: 'a'.repeat(64) },
      ] } },
    { id: 'pause-file-answer', type: 'text', position: 'left', conversation_id: conversationId, message_id: messageId(522), msg_id: messageId(522), turn_id: turnId,
      created_at: 200, content: { content: 'The image is ready.' } },
  ];
  const Timeline = () => {
    const [phase, setPhase] = useState<'paused' | 'resumed' | 'finished'>('paused');
    const update = useUpdateMessageList();
    const pause = { turnId, reason: 'EXECUTION_USER_REQUESTED', cleanupProven: true, pausedAt: 250 };
    return <ConversationProvider value={{ conversation_id: conversationId, workspace: '/workspace', type: 'nomi', readOnly: true,
      isProcessing: phase === 'resumed', isTurnStateHydrated: true, activeTurnId: turnId,
      executionPause: phase === 'resumed' ? null : pause }}>
      <MessageList />
      <button onClick={() => update(current => [...current, { id: 'invalid-file-finish', type: 'agent_status', conversation_id: conversationId,
        message_id: messageId(523), msg_id: messageId(523), turn_id: turnId, created_at: 260,
        content: { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed', finished_at_ms: Number.NaN } }])}>Invalid finish</button>
      <button onClick={() => setPhase('resumed')}>Resume files</button>
      <button onClick={() => {
        update(current => [...current, { id: 'canonical-file-finish', type: 'agent_status', conversation_id: conversationId,
          message_id: messageId(524), msg_id: messageId(524), turn_id: turnId, created_at: 300,
          content: { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed', started_at_ms: 100, finished_at_ms: 300 } }]);
        setPhase('finished');
      }}>Finish files</button>
    </ConversationProvider>;
  };
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='paused-canonical-turn-files-test' subscribeGlobalOpen={false}>
      <MessageListProvider initialValue={initial}><Timeline /></MessageListProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>);
  expect(query).not.toHaveBeenCalled();
  expect(page.queryByTestId('turn-deliverables-card')).toBeNull();
  expect(page.queryByTestId('turn-deliverables-images')).toBeNull();
  fireEvent.click(page.getByText('Invalid finish'));
  expect(query).not.toHaveBeenCalled();
  expect(page.queryByTestId('turn-deliverables-card')).toBeNull();
  fireEvent.click(page.getByText('Resume files'));
  expect(query).not.toHaveBeenCalled();
  expect(page.queryByTestId('turn-deliverables-images')).toBeNull();
  fireEvent.click(page.getByText('Finish files'));
  await waitFor(() => expect(page.getByText('This turn involves 1 files')).toBeTruthy());
  expect(page.getByText('report.docx')).toBeTruthy();
  expect(page.getByTestId('turn-deliverables-images')).toBeTruthy();
  expect(query).toHaveBeenCalledWith({ conversation_id: conversationId, turn_ids: [turnId] });
});

test('canonical group headings span model waits without changing tool outcomes or crossing narration and terminal boundaries', () => {
  const groupId = 'canonical-model-wait';
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const tool: IMessageToolCall = { ...base, id: 'wait-completed-tool', msg_id: messageId(811), type: 'tool_call', created_at: 10,
    content: { call_id: 'wait-read', name: 'read_file', status: 'completed', args: { path: 'source.ts' }, output: 'Source available.',
      timeline: { through_seq: 20, segments: [{ segment_id: 'wait-read-segment', group_id: groupId, first_seq: 10, last_seq: 20,
        kind: 'tool', state: 'completed', call_id: 'wait-read' }] } } };
  const narration: TMessage = { ...base, id: 'wait-narration', msg_id: messageId(813), type: 'text', created_at: 30,
    content: { content: 'The source is available.', timeline: { through_seq: 30, segments: [{ segment_id: 'wait-public-boundary',
      group_id: groupId, first_seq: 30, last_seq: 30, kind: 'narration', state: 'completed', start_utf16: 0, end_utf16: 24 }] } } };
  const staleTool: IMessageToolCall = { ...base, id: 'wait-stale-tool', msg_id: messageId(814), type: 'tool_call', created_at: 40,
    content: { call_id: 'wait-stale-read', name: 'read_file', status: 'running', args: { path: 'next.ts' },
      timeline: { through_seq: 40, segments: [{ segment_id: 'wait-stale-segment', group_id: 'canonical-stale-tail',
        first_seq: 40, last_seq: 40, kind: 'tool', state: 'running', call_id: 'wait-stale-read' }] } } };
  const initial: TMessage[] = [
    { ...base, id: 'wait-user', msg_id: turnId, type: 'text', position: 'right', created_at: 1, content: { content: 'Inspect the source.' } },
    tool,
    { ...base, id: 'wait-model-preparing', msg_id: messageId(812), type: 'agent_status', created_at: 21,
      content: { backend: 'nomi', status: 'preparing' } },
  ];
  const StreamControls = () => {
    const update = useUpdateMessageList();
    return <>
      <button onClick={() => update(current => current.map(message => message.id === tool.id
        ? { ...tool, content: { ...tool.content, output: 'Source available. Stream receipt refreshed.' } } : message))}>Refresh completed tool</button>
      <button onClick={() => update(current => [...current, narration, staleTool])}>Deliver phase narration</button>
      <button onClick={() => update(current => [...current, { ...base, id: 'wait-terminal', msg_id: messageId(815),
        type: 'agent_status', created_at: 50, content: { backend: 'nomi', status: 'prepared', turn_summary: true,
          turn_state: 'completed', started_at_ms: 1, finished_at_ms: 50 } }])}>Commit terminal receipt</button>
    </>;
  };
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='canonical-group-model-wait' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true, isProcessing: true,
        isTurnStateHydrated: true, activeTurnId: turnId, activeRequestMessageId: turnId }}>
        <MessageListProvider initialValue={initial}><MessageList /><StreamControls /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>);
  const group = page.container.querySelector(`[data-process-group-id="${groupId}"]`)!;
  const toggle = group.querySelector('button')!;
  expect(group.classList.contains('turn-process-group--running')).toBe(true);
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(toggle);
  expect(group.querySelector('[data-tool-call-id="wait-read"] .turn-process-trace__row--completed')).not.toBeNull();
  expect(group.querySelector('.turn-process-trace__row--running')).toBeNull();
  fireEvent.click(toggle);
  fireEvent.click(page.getByRole('button', { name: 'Refresh completed tool' }));
  expect(page.container.querySelector(`[data-process-group-id="${groupId}"]`)).toBe(group);
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(group.classList.contains('turn-process-group--running')).toBe(true);
  fireEvent.click(toggle);
  expect(group.querySelector('[data-tool-call-id="wait-read"] .turn-process-trace__row--completed')).not.toBeNull();
  expect(group.querySelector('.turn-process-trace__row--running')).toBeNull();
  fireEvent.click(page.getByRole('button', { name: 'Deliver phase narration' }));
  expect(group.classList.contains('turn-process-group--running')).toBe(false);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(page.container.querySelector('.turn-process-disclosure--live')).not.toBeNull();
  expect(page.container.querySelector('[data-process-segment-id="wait-public-boundary"]')?.closest('.turn-process-group')).toBeNull();
  const nextGroup = page.container.querySelector('[data-process-group-id="canonical-stale-tail"]')!;
  expect(nextGroup.querySelector('button')?.getAttribute('aria-expanded')).toBe('false');
  expect(nextGroup.querySelector('.turn-process-group__body')).toBeNull();
  expect(nextGroup.classList.contains('turn-process-group--running')).toBe(true);
  fireEvent.click(page.getByRole('button', { name: 'Commit terminal receipt' }));
  expect(page.container.querySelector('.turn-process-disclosure--live')).toBeNull();
  fireEvent.click(page.getByRole('button', { name: messagesLocale.turnProcess.expand }));
  expect(page.container.querySelector('[data-process-group-id="canonical-stale-tail"]')).not.toBeNull();
  expect(page.container.querySelector('.turn-process-group--running')).toBeNull();
  expect(staleTool.content.status).toBe('running');
});

test('canonical segment references keep active groups collapsed and preserve manual choices around narration', () => {
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const timeline = (segments: MessageTimelineSegment[]): MessageTimeline => ({ through_seq: 30, segments });
  const ref = (id: string, group: string, seq: number, kind: MessageTimelineSegment['kind'], start = 0, end = 1,
    state: MessageTimelineSegment['state'] = 'completed'): MessageTimelineSegment => ({
    segment_id: id, group_id: group, first_seq: seq, last_seq: seq, kind, state, start_utf16: start, end_utf16: end,
  });
  const messages: TMessage[] = [
    { ...base, id: 'segment-user', msg_id: turnId, type: 'text', position: 'right', created_at: 1, content: { content: 'Inspect the files' } },
    { ...base, id: 'segment-thinking', msg_id: messageId(801), type: 'thinking', created_at: 2, content: { content: 'First.Second.', status: 'thinking', timeline: timeline([
      ref('reasoning-first', 'canonical-first', 2, 'reasoning', 0, 6), ref('reasoning-second', 'canonical-second', 8, 'reasoning', 6, 13, 'running'),
    ]) } } as TMessage,
    { ...base, id: 'segment-tool', msg_id: messageId(802), type: 'tool_call', created_at: 3, content: { call_id: 'segment-read', name: 'read_file', status: 'completed', args: { path: 'source.ts' },
      timeline: timeline([{ ...ref('tool-first', 'canonical-first', 4, 'tool'), call_id: 'segment-read' }]) } } as TMessage,
    { ...base, id: 'segment-progress', msg_id: messageId(803), type: 'text', created_at: 4, content: { content: 'The source is available.', timeline: timeline([
      ref('public-between', 'canonical-first', 6, 'narration', 0, 24),
    ]) } } as TMessage,
    { ...base, id: 'segment-final', msg_id: messageId(804), type: 'text', created_at: 9, content: { content: 'Final deliverable remains outside.' } },
    { ...base, id: 'segment-summary', msg_id: messageId(805), type: 'agent_status', created_at: 10, content: { backend: 'nomi', status: 'prepared', turn_summary: true,
      turn_state: 'completed', turn_outcome: { kind: 'answered', message_ids: [messageId(804)] } } },
  ];
  const StreamControls = () => {
    const update = useUpdateMessageList();
    return <button onClick={() => update(current => current.map(message => message.id === 'segment-thinking' && message.type === 'thinking'
      ? { ...message, content: { ...message.content, content: 'First.Second. Streamed.', timeline: timeline([
        ref('reasoning-first', 'canonical-first', 2, 'reasoning', 0, 6),
        ref('reasoning-second', 'canonical-second', 8, 'reasoning', 6, 23, 'running'),
      ]) } } : message))}>Update current reasoning</button>;
  };
  const view = (running: boolean) => <MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='canonical-segments-integration' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', readOnly: true, isProcessing: running }}>
        <MessageListProvider initialValue={messages}><MessageList /><StreamControls /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>;
  const page = render(view(true));
  const first = page.container.querySelector('[data-process-group-id="canonical-first"]')!;
  const second = page.container.querySelector('[data-process-group-id="canonical-second"]')!;
  const firstToggle = first.querySelector('button')!;
  const secondToggle = second.querySelector('button')!;
  expect(firstToggle.getAttribute('aria-expanded')).toBe('false');
  expect(secondToggle.getAttribute('aria-expanded')).toBe('false');
  expect(first.querySelector('.turn-process-group__body')).toBeNull();
  expect(second.querySelector('.turn-process-group__body')).toBeNull();
  expect(second.classList.contains('turn-process-group--running')).toBe(true);
  const narration = page.container.querySelector('[data-process-segment-id="public-between"] [data-testid="process-narration"]')!;
  expect(narration).not.toBeNull();
  expect(narration.closest('.turn-process-group')).toBeNull();
  expect(first.compareDocumentPosition(narration) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(narration.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  fireEvent.click(firstToggle);
  fireEvent.click(page.getByRole('button', { name: 'Update current reasoning' }));
  expect(firstToggle.getAttribute('aria-expanded')).toBe('true');
  expect(secondToggle.getAttribute('aria-expanded')).toBe('false');
  expect(second.querySelector('[data-process-segment-id="reasoning-second"]')).toBeNull();
  fireEvent.click(secondToggle);
  expect(second.querySelector('[data-process-segment-id="reasoning-second"]')).not.toBeNull();
  expect(second.querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain('Second. Streamed.');
  page.rerender(view(false));
  expect(page.container.querySelector('.turn-process-disclosure__body')).toBeNull();
  const answer = page.container.querySelector('#message-segment-final')!;
  expect(answer).not.toBeNull();
  expect(answer.closest('.turn-process-disclosure')).toBeNull();
  act(() => dispatchChatMessageJump({ conversation_id: conversationId, messageId: messageId(802) }));
  expect(page.container.querySelector('[data-process-group-id="canonical-first"] > button')?.getAttribute('aria-expanded')).toBe('true');
  expect(page.container.querySelector('[data-process-group-id="canonical-second"] > button')?.getAttribute('aria-expanded')).toBe('false');
  expect(page.container.querySelector('[data-tool-call-id="segment-read"]')).not.toBeNull();
});
