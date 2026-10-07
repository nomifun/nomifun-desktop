import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { ipcBridge } from '@/common';
import type { IMessageToolCall, TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId, parsePersistedArtifactId, parseProviderId } from '@/common/types/ids';
import { ConversationCreationTasksProvider } from '@/renderer/creation/ConversationCreationTasks';
import * as creationClient from '@/renderer/creation/client';
import type { ConversationCreationTask } from '@/renderer/creation/types';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { creativeAssetClient } from '@/renderer/pages/creativeStudio/assets/client';
import type { CreativeAsset } from '@/renderer/pages/creativeStudio/assets/types';
import messagesLocale from '@/renderer/services/i18n/locales/en-US/messages.json';
import { PreviewProvider } from '../Preview';
import MessageList from './MessageList';
import { MessageListProvider } from './hooks';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US', resources: { 'en-US': { translation: { messages: messagesLocale } } },
});

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000301');
const requestId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000302');
const replyId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000303');
const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000304');
const assetId = '0190f5fe-7c00-7a00-8000-000000000305';
const taskId = '0190f5fe-7c00-7a00-8000-000000000306';
const workspace = '/workspace/creation-placement-test';
const createdAt = 1_700_000_000_000;

const request: TMessage = {
  id: 'media-request', message_id: requestId, msg_id: requestId, conversation_id: conversationId,
  type: 'text', position: 'right', created_at: createdAt, content: { content: 'Generate a cat image' },
};
const reply: TMessage = {
  id: 'media-reply', message_id: replyId, msg_id: replyId, turn_id: requestId, conversation_id: conversationId,
  type: 'text', position: 'left', created_at: createdAt + 1000, content: { content: 'The cat image task has been submitted.' },
};

function task(overrides: Partial<ConversationCreationTask> = {}): ConversationCreationTask {
  return {
    creation_task_id: taskId, owner: { kind: 'conversation_turn', conversation_id: conversationId, message_id: requestId },
    provider_id: providerId, model: 'image-model', capability: 't2i', params: { prompt: 'cat', size: '1024x1024' },
    inputs: [], status: 'succeeded', error: null, result_asset_ids: [assetId], submitted_at: createdAt,
    started_at: createdAt, finished_at: createdAt + 2000, ...overrides,
  };
}

const asset: CreativeAsset = {
  id: assetId, kind: 'image', title: 'Generated cat', collection: null, tags: [], mimeType: 'image/png',
  width: 1024, height: 1024, bytes: 4, inLibrary: true, textContent: null, origin: null,
  originalUrl: '/generated-cat.png', thumbnailUrl: null, createdAt, updatedAt: createdAt,
};

let restoreMocks = () => {};
beforeEach(() => {
  const content = spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockImplementation(() => () => {});
  const tree = spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockImplementation(() => () => {});
  const entry = spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockImplementation(() => () => {});
  const watchStart = spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(undefined);
  const watchStop = spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(undefined);
  const watchFiles = spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockImplementation(() => () => {});
  const files = spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
  const completed = spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(() => () => {});
  const tasks = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task()]);
  const result = spyOn(creativeAssetClient, 'get').mockResolvedValue(asset);
  restoreMocks = () => {
    content.mockRestore(); tree.mockRestore(); entry.mockRestore(); watchStart.mockRestore(); watchStop.mockRestore();
    watchFiles.mockRestore(); files.mockRestore(); completed.mockRestore(); tasks.mockRestore(); result.mockRestore();
  };
});
afterEach(() => { cleanup(); restoreMocks(); });

function timeline(messages: TMessage[], { readOnly = false }: { readOnly?: boolean } = {}) {
  const cache = new Map();
  return (running: boolean) => <MemoryRouter><I18nextProvider i18n={i18n}>
    <SWRConfig value={{ provider: () => cache, shouldRetryOnError: false }}>
      <PreviewProvider persistNamespace='creation-turn-placement-test' subscribeGlobalOpen={false}>
        <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', workspace, readOnly, isProcessing: running }}>
          <ConversationCreationTasksProvider conversationId={conversationId} enabled>
            <MessageListProvider initialValue={messages}><MessageList /></MessageListProvider>
          </ConversationCreationTasksProvider>
        </ConversationProvider>
      </PreviewProvider>
    </SWRConfig>
  </I18nextProvider></MemoryRouter>;
}

function expectBefore(before: Element, after: Element) {
  expect(Boolean(before.compareDocumentPosition(after) & Node.DOCUMENT_POSITION_FOLLOWING)).toBe(true);
}

function expectSingleReplyFooter(page: ReturnType<typeof render>) {
  const footers = page.getAllByTestId('turn-actions');
  expect(footers).toHaveLength(1);
  const footer = footers[0];
  expect(footer.querySelectorAll('[data-testid="message-actions"]')).toHaveLength(1);
  expect(footer.querySelectorAll('[data-testid="message-copy-action"]')).toHaveLength(1);
  expect(footer.querySelector('[data-testid="message-actions"] > span')?.textContent?.trim()).toBeTruthy();
  const assistant = page.getByTestId('message-text-left');
  expect(assistant.querySelector('[data-testid="message-actions"]')).toBeNull();
  // The input retains its own copy/time row; only the reply's footer moves.
  expect(page.getByTestId('message-text-right').querySelectorAll('[data-testid="message-actions"]')).toHaveLength(1);
  return { assistant, footer };
}

test('media from a completed turn appears after its reply and before the only reply copy/time footer', async () => {
  const page = render(timeline([request, reply])(false));
  const media = await page.findByRole('button', { name: '预览图片：Generated cat' });
  const { assistant, footer } = expectSingleReplyFooter(page);
  expect(assistant.querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain(reply.content.content);
  expectBefore(assistant, media);
  expectBefore(media, footer);
});

test('mixed file deliverables and multiple media tasks share one footer in a read-only historical turn', async () => {
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([
    task(), task({ creation_task_id: '0190f5fe-7c00-7a00-8000-000000000307', status: 'queued', result_asset_ids: [], finished_at: null }),
  ]);
  const file: IMessageToolCall = {
    id: 'media-file-result', message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000308'),
    msg_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000308'), turn_id: requestId,
    conversation_id: conversationId, position: 'left', type: 'tool_call', created_at: createdAt + 500,
    content: { call_id: 'write-report', name: 'write_file', status: 'completed', args: { path: 'outputs/report.txt' },
      output: 'Report written', artifacts: [{ id: parsePersistedArtifactId('0190f5fe-7c00-7a00-8000-000000000309'),
        kind: 'file', mime_type: 'text/plain', path: `${workspace}/outputs/report.txt`, relative_path: 'outputs/report.txt',
        size_bytes: 42, sha256: 'a'.repeat(64) }] },
  };
  const page = render(timeline([request, file, reply], { readOnly: true })(false));
  await page.findByRole('button', { name: '预览图片：Generated cat' });
  await waitFor(() => expect(page.container.querySelector('[data-deliverable-path="outputs/report.txt"]')).not.toBeNull());
  const { assistant, footer } = expectSingleReplyFooter(page);
  const files = page.getByTestId('turn-deliverables');
  const media = page.container.querySelectorAll('[data-creation-task]');
  expect(media).toHaveLength(2);
  expectBefore(assistant, files);
  expectBefore(files, media[0]);
  media.forEach(card => expectBefore(card, footer));
  expect(page.queryByRole('button', { name: '取消任务' })).toBeNull();
  expect(page.queryByRole('button', { name: '再次创作' })).toBeNull();
});

test('an ongoing turn shows its real media task without prematurely displaying the reply footer', async () => {
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task({ status: 'running', result_asset_ids: [], finished_at: null })]);
  const view = timeline([request, reply]);
  const page = render(view(true));
  await waitFor(() => expect(page.container.querySelector('[data-creation-task]')).not.toBeNull());
  expect(page.getByText('生成中')).toBeTruthy();
  expect(page.queryByTestId('turn-actions')).toBeNull();
  // Streaming narration stays inside the expanded process journal.
  expect(page.queryByTestId('message-text-left')).toBeNull();
  expect(page.container.querySelectorAll('[data-testid="message-copy-action"]')).toHaveLength(1);
  page.rerender(view(false));
  const { assistant, footer } = expectSingleReplyFooter(page);
  const media = page.container.querySelector('[data-creation-task]')!;
  expectBefore(assistant, media);
  expectBefore(media, footer);
});

test('direct generation with no assistant reply keeps the user input and shows the media without an invented reply footer', async () => {
  const page = render(timeline([request], { readOnly: true })(false));
  const media = await page.findByRole('button', { name: '预览图片：Generated cat' });
  const user = page.getByTestId('message-text-right');
  expectBefore(user, media);
  expect(user.querySelectorAll('[data-testid="message-actions"]')).toHaveLength(1);
  expect(page.queryByTestId('message-text-left')).toBeNull();
  expect(page.queryByTestId('turn-actions')).toBeNull();
  expect(page.container.querySelectorAll('[data-testid="message-copy-action"]')).toHaveLength(1);
});
