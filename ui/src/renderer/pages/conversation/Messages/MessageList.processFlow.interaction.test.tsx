import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageToolCall, TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { PreviewProvider } from '../Preview';
import MessageList from './MessageList';
import { MessageListProvider, useUpdateMessageList } from './hooks';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';
import { ipcBridge } from '@/common';
import agentExecutionLocale from '@/renderer/services/i18n/locales/en-US/agentExecution.json';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { ExecutionProvider, useExecutionSafe } from '../execution/ExecutionContext';
import { executionId, leadConversation, leadConversationId, makeAttempt, makeDetail, makeStep, requestId } from '../../../../../test/fixtures/conversationDelegation';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages: messagesLocale, agentExecution: agentExecutionLocale } } } });
const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000061');
const turnId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000062');
const messageId = (index: number) => parseMessageId(`0190f5fe-7c00-7a00-8000-${String(index).padStart(12, '0')}`);

let restoreListeners = () => {};
beforeEach(() => {
  const content = spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  const tree = spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  const entry = spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
  const metadata = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockImplementation(async ({ path }) => ({
    name: 'app.ts', path, size: 42, type: 'text/plain', lastModified: 0, isDirectory: false,
  }));
  const watchStart = spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(undefined);
  const watchStop = spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(undefined);
  const watchFiles = spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockImplementation(() => () => {});
  const files = spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
  restoreListeners = () => {
    content.mockRestore(); tree.mockRestore(); entry.mockRestore(); metadata.mockRestore();
    watchStart.mockRestore(); watchStop.mockRestore(); watchFiles.mockRestore(); files.mockRestore();
  };
});

afterEach(() => { cleanup(); restoreListeners(); });

test('a live journal only animates its current thought and preserves completed thought disclosures', () => {
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

test('canonical terminal metadata closes stale thinking while the session still reports processing', () => {
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const messages: TMessage[] = [
    { ...base, id: 'user', message_id: turnId, msg_id: turnId, type: 'text', created_at: 1, position: 'right', content: { content: 'Inspect the files' } },
    { ...base, id: 'stale-thought', msg_id: messageId(2), type: 'thinking', created_at: 2, content: { content: 'The source inspection reasoning.', status: 'thinking' } },
    { ...base, id: 'final', msg_id: messageId(3), type: 'text', created_at: 3, content: { content: 'The inspection is complete.' } },
    { ...base, id: 'terminal-metadata', msg_id: messageId(4), type: 'agent_status', created_at: 4,
      content: { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed', started_at_ms: 1, finished_at_ms: 3 } },
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
  expect(page.container.querySelector('.turn-process-trace__row--current-activity')).not.toBeNull();
  expect(page.queryByTestId('conversation-current-activity')).toBeNull();
  fireEvent.click(page.getByRole('button', { name: messagesLocale.turnProcess.collapse }));
  expect(page.container.querySelector('.turn-process-disclosure__body')).toBeNull();
  expect(page.queryByTestId('conversation-current-activity')).toBeNull();
});

test('a completed journal defaults closed and expands its full reasoning and calls without hiding the final reply', () => {
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const firstCall: IMessageToolCall = { ...base, id: 'first-call', message_id: messageId(3), msg_id: messageId(3), created_at: 3,
    type: 'tool_call', content: { call_id: 'read-1', name: 'read_file', status: 'error', args: { path: 'src/first.ts' },
      output: 'First attempt failed', artifacts: [], retry: { retry_group_id: 'read-1', attempt_no: 1 } } };
  const retry: IMessageToolCall = { ...firstCall, id: 'retry-call', message_id: messageId(6), msg_id: messageId(6), created_at: 6,
    content: { ...firstCall.content, call_id: 'read-2', status: 'completed', output: 'Second attempt result',
      retry: { retry_group_id: 'read-1', retry_of_call_id: 'read-1', attempt_no: 2 } } };
  const messages: TMessage[] = [
    { ...base, id: 'user', message_id: turnId, msg_id: turnId, type: 'text', created_at: 1, position: 'right', content: { content: 'Inspect the files' } },
    { ...base, id: 'thought-1', msg_id: messageId(2), type: 'thinking', created_at: 2, content: { content: '**First reasoning**\n\nInspect the source.', status: 'done' } },
    firstCall,
    { ...base, id: 'progress', msg_id: messageId(4), type: 'text', created_at: 4, content: { content: 'I will retry with the available source.' } },
    { ...base, id: 'thought-2', msg_id: messageId(5), type: 'thinking', created_at: 5, content: { content: 'Second reasoning', status: 'done' } },
    retry,
    { ...base, id: 'final', msg_id: messageId(7), type: 'text', created_at: 7, content: { content: 'The inspection is complete.' } },
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
  thoughtHeaders.forEach((header) => expect(header.getAttribute('aria-expanded')).toBe('true'));
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
