import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { IMessageToolCall, IMessageToolGroup, TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { PreviewProvider } from '../Preview';
import MessageList from './MessageList';
import { MessageListProvider } from './hooks';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';
import { ipcBridge } from '@/common';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages: messagesLocale } } } });
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

test('legacy writes remain individual calls while the final artifact card stays aggregated', async () => {
  const base = { conversation_id: conversationId, turn_id: turnId, position: 'left' as const };
  const writes = [1, 2].map<IMessageToolGroup>((index) => ({
    ...base, id: `write-${index}`, message_id: messageId(index + 20), msg_id: messageId(index + 20),
    created_at: index + 1, type: 'tool_group', content: [{
      call_id: `write-call-${index}`, name: 'WriteFile', description: 'src/app.ts',
      status: 'Success', render_output_as_markdown: false,
      result_display: { file_name: '/process-write-fixture/src/app.ts', file_diff: `diff --git a/src/app.ts b/src/app.ts\n--- a/src/app.ts\n+++ b/src/app.ts\n@@ -1 +1 @@\n-old\n+revision-${index}\n` },
    }],
  }));
  const originalContent = structuredClone(writes.map((write) => write.content));
  const messages: TMessage[] = [
    { ...base, id: 'write-user', message_id: turnId, msg_id: turnId, type: 'text', position: 'right', created_at: 1, content: { content: 'Update the file' } },
    ...writes,
    { ...base, id: 'write-final', msg_id: messageId(23), type: 'text', created_at: 4, content: { content: 'The file is updated.' } },
  ];
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='process-write-test' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', workspace: '/process-write-fixture', readOnly: true, isProcessing: false }}>
        <MessageListProvider initialValue={messages}><MessageList /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>);
  expect(page.container.querySelector('.turn-process-disclosure__body')).toBeNull();
  fireEvent.click(page.getByRole('button', { name: messagesLocale.turnProcess.expand }));
  expect(Array.from(page.container.querySelectorAll('[data-tool-call-id]'), (call) => call.getAttribute('data-tool-call-id')))
    .toEqual(['write-call-1', 'write-call-2']);
  expect(page.getAllByTestId('turn-deliverables')).toHaveLength(1);
  await waitFor(() => expect(page.getByTestId('turn-deliverables').textContent).toContain('app.ts'));
  expect(writes[0].content[0].status).toBe('Success');
  expect(writes.map((write) => write.content)).toEqual(originalContent);
});
afterEach(() => { cleanup(); restoreListeners(); });

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
