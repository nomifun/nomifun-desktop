import '../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig, useSWRConfig } from 'swr';
import { parseConversationId, parseMessageId, parseProviderId, type ConversationId } from '@/common/types/ids';
import { conversation as conversationEvents, type IConversationTurnCompletedEvent } from '@/common/adapter/ipcBridge';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { creativeAssetClient } from '@/renderer/pages/creativeStudio/assets/client';
import type { CreativeAsset } from '@/renderer/pages/creativeStudio/assets/types';
import { CreationComposerContext, type CreationComposerValue } from './CreationComposerContext';
import { ConversationCreationTaskCards, ConversationCreationTasksProvider, ConversationImagePreview } from './ConversationCreationTasks';
import * as creationClient from './client';
import { creationModeFor, type ConversationCreationTask, type CreationTaskCapability } from './types';
import { emptyCreationDraft } from './useCreationDraft';
import { recallCreationTask } from './recallTask';
import { emitter } from '@/renderer/utils/emitter';
import conversation from '@/renderer/services/i18n/locales/zh-CN/conversation.json';
import common from '@/renderer/services/i18n/locales/zh-CN/common.json';

const completedListeners = new Set<(event: IConversationTurnCompletedEvent) => void>();
beforeEach(() => {
  completedListeners.clear();
  spyOn(conversationEvents.turnCompleted, 'on').mockImplementation(listener => {
    completedListeners.add(listener);
    return () => { completedListeners.delete(listener); };
  });
});
afterEach(() => { cleanup(); mock.restore(); });

const testI18n = createInstance();
await testI18n.use(initReactI18next).init({
  lng: 'zh-CN',
  fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: { conversation, common } } },
  interpolation: { escapeValue: false },
});

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000102');
const messageId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000106');
const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000105');
const assetId = '0190f5fe-7c00-7a00-8000-000000000107';

function task(overrides: Partial<ConversationCreationTask> = {}): ConversationCreationTask {
  return {
    creation_task_id: '0190f5fe-7c00-7a00-8000-000000000108',
    owner: { kind: 'conversation_turn', conversation_id: conversationId, message_id: messageId },
    provider_id: providerId, model: 'image-model', capability: 't2i', params: { prompt: 'cat' }, inputs: [],
    status: 'running', error: null, result_asset_ids: [], submitted_at: 0, started_at: 0, finished_at: null,
    ...overrides,
  };
}

function asset(kind: CreativeAsset['kind'] = 'image'): CreativeAsset {
  return {
    id: assetId, kind, title: '产物', collection: null, tags: [], mimeType: null,
    width: null, height: null, bytes: 4, inLibrary: true, textContent: kind === 'text' ? '生成的文本' : null,
    origin: null, originalUrl: '/artifact.png', thumbnailUrl: null, createdAt: 0, updatedAt: 0,
  };
}

function RefreshTaskState({ id }: { id: ConversationId }) {
  const { mutate } = useSWRConfig();
  return <button type='button' onClick={() => void mutate(creationClient.creationTasksKey(id))}>刷新测试任务</button>;
}

function TasksHarness({ enabled = true, readOnly = false, composer = null, refreshControl = false, id = conversationId }: {
  enabled?: boolean; readOnly?: boolean; composer?: CreationComposerValue | null; refreshControl?: boolean;
  id?: ConversationId;
} = {}) {
  return (
    <I18nextProvider i18n={testI18n}>
      <SWRConfig value={{ provider: () => new Map(), shouldRetryOnError: false }}>
        <CreationComposerContext.Provider value={composer}>
          <ConversationProvider value={{ conversation_id: id, type: 'nomi', readOnly }}>
            <ConversationCreationTasksProvider conversationId={id} enabled={enabled}>
              <ConversationCreationTaskCards messageId={messageId} />
            </ConversationCreationTasksProvider>
            {refreshControl && <RefreshTaskState id={id} />}
          </ConversationProvider>
        </CreationComposerContext.Provider>
      </SWRConfig>
    </I18nextProvider>
  );
}

function renderTasks(options: Parameters<typeof TasksHarness>[0] = {}) {
  return render(<TasksHarness {...options} />);
}

function completedEvent(id: ConversationId = conversationId): IConversationTurnCompletedEvent {
  return {
    conversation_id: id, turn_id: messageId, status: 'finished', state: 'ai_waiting_input', detail: '',
    can_send_message: true, workspace: '', model: { platform: '', name: '', use_model: '' },
    last_message: { content: '', created_at: 0 },
    runtime: { state: 'idle', can_send_message: true, has_runtime: true, is_processing: false },
  };
}

function composer(): CreationComposerValue {
  return { draft: emptyCreationDraft(), update: mock(() => {}), setMode: mock(() => {}), selectMode: mock(() => {}), exit: mock(() => {}) };
}

test('an authorized chat-only Agent reads and cancels its generation task without a creation composer', async () => {
  const current = task();
  const list = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([current]);
  const cancel = spyOn(creationClient, 'cancelCreation').mockImplementation(async () => {
    list.mockResolvedValue([{ ...current, status: 'canceled' }]);
    return { ...current, status: 'canceled' };
  });
  const page = renderTasks();
  expect(await page.findByText('图片生成')).toBeTruthy();
  expect(list).toHaveBeenCalledWith(conversationId);
  fireEvent.click(page.getByRole('button', { name: '取消任务' }));
  await waitFor(() => expect(cancel).toHaveBeenCalledWith(conversationId, current.creation_task_id));
  expect(await page.findByText('已取消')).toBeTruthy();
});

test('a Session without media creation authority never queries generation tasks', async () => {
  const list = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task()]);
  const page = renderTasks({ enabled: false, composer: composer() });
  await waitFor(() => expect(page.container.querySelector('[data-creation-task]')).toBeNull());
  expect(list).not.toHaveBeenCalled();
  act(() => {
    emitter.emit('conversation.turn.settled', conversationId);
    completedListeners.forEach(listener => listener(completedEvent()));
  });
  expect(completedListeners.size).toBe(0);
  expect(list).not.toHaveBeenCalled();
});

test.each(['settled', 'completed'] as const)('%s lifecycle notifications immediately refresh an empty task pool in a read-only Session', async signal => {
  const list = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([]);
  const cancel = spyOn(creationClient, 'cancelCreation');
  const page = renderTasks({ readOnly: true });
  await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
  await act(async () => {});
  expect(page.container.querySelector('[data-creation-task]')).toBeNull();
  list.mockResolvedValue([task()]);
  act(() => {
    if (signal === 'settled') emitter.emit('conversation.turn.settled', conversationId);
    else completedListeners.forEach(listener => listener(completedEvent()));
  });
  expect(await page.findByRole('group', { name: '图片生成：生成中' })).toBeTruthy();
  expect(list).toHaveBeenCalledTimes(2);
  expect(cancel).not.toHaveBeenCalled();
});

test('lifecycle notifications for another Session or a non-idle runtime do not refresh tasks', async () => {
  const list = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([]);
  renderTasks();
  await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
  await act(async () => {});
  const otherId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000110');
  const event = completedEvent();
  act(() => {
    emitter.emit('conversation.turn.settled', otherId);
    completedListeners.forEach(listener => {
      listener(completedEvent(otherId));
      listener({ ...event, runtime: { ...event.runtime, is_processing: true } });
      listener({ ...event, runtime: { ...event.runtime, active_turn_id: messageId } });
    });
  });
  expect(list).toHaveBeenCalledTimes(1);
});

test('completion and settled signals coalesce while their generation task read is pending', async () => {
  const list = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([]);
  const page = renderTasks();
  await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
  await act(async () => {});
  let resolveRead: ((tasks: ConversationCreationTask[]) => void) | undefined;
  list.mockImplementation(() => new Promise(resolve => { resolveRead = resolve; }));
  act(() => {
    completedListeners.forEach(listener => listener(completedEvent()));
    emitter.emit('conversation.turn.settled', conversationId);
    completedListeners.forEach(listener => listener(completedEvent()));
  });
  await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  expect(page.container.querySelector('[data-creation-task]')).toBeNull();
  await act(async () => { resolveRead!([task()]); });
  expect(await page.findByRole('group', { name: '图片生成：生成中' })).toBeTruthy();
  expect(list).toHaveBeenCalledTimes(2);
});

test('changing Sessions disposes old lifecycle callbacks and refreshes only the new Session', async () => {
  const list = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([]);
  const page = renderTasks();
  await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
  await act(async () => {});
  const previousListeners = [...completedListeners];
  const otherId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000110');
  page.rerender(<TasksHarness id={otherId} />);
  await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  await act(async () => {});
  act(() => {
    emitter.emit('conversation.turn.settled', conversationId);
    completedListeners.forEach(listener => listener(completedEvent()));
    previousListeners.forEach(listener => listener(completedEvent()));
  });
  expect(list).toHaveBeenCalledTimes(2);
  list.mockResolvedValue([task({ owner: { kind: 'conversation_turn', conversation_id: otherId, message_id: messageId } })]);
  act(() => { emitter.emit('conversation.turn.settled', otherId); });
  expect(await page.findByRole('group', { name: '图片生成：生成中' })).toBeTruthy();
  expect(list).toHaveBeenLastCalledWith(otherId);
  expect(list).toHaveBeenCalledTimes(3);
});

test.each(['disable', 'unmount'] as const)('%s removes lifecycle listeners and fences callbacks already retained by the transport', async action => {
  const list = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([]);
  const page = renderTasks();
  await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
  await act(async () => {});
  const previousListeners = [...completedListeners];
  if (action === 'disable') page.rerender(<TasksHarness enabled={false} />);
  else page.unmount();
  expect(completedListeners.size).toBe(0);
  act(() => {
    emitter.emit('conversation.turn.settled', conversationId);
    previousListeners.forEach(listener => listener(completedEvent()));
  });
  expect(list).toHaveBeenCalledTimes(1);
});

test('read-only transcripts display ongoing and completed artifacts without task or composer effects', async () => {
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([
    task(), task({ creation_task_id: '0190f5fe-7c00-7a00-8000-000000000109', status: 'succeeded', result_asset_ids: [assetId] }),
  ]);
  spyOn(creativeAssetClient, 'get').mockResolvedValue(asset());
  const cancel = spyOn(creationClient, 'cancelCreation');
  const creation = composer();
  const page = renderTasks({ readOnly: true, composer: creation });
  expect(await page.findByRole('button', { name: '预览图片：产物' })).toBeTruthy();
  expect(page.getByText('生成中')).toBeTruthy();
  expect(page.getByText('已完成')).toBeTruthy();
  for (const name of ['取消任务', '再次创作', '编辑图片', '转为视频']) {
    expect(page.queryByRole('button', { name })).toBeNull();
  }
  expect(page.getByRole('button', { name: '另存为' })).toBeTruthy();
  expect(cancel).not.toHaveBeenCalled();
  expect(creation.update).not.toHaveBeenCalled();
  expect(creation.selectMode).not.toHaveBeenCalled();
});

test('completed images remain visible without a composer and do not offer unavailable editing actions', async () => {
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task({ status: 'succeeded', result_asset_ids: [assetId] })]);
  spyOn(creativeAssetClient, 'get').mockResolvedValue(asset());
  const page = renderTasks();
  expect(await page.findByRole('button', { name: '预览图片：产物' })).toBeTruthy();
  for (const name of ['再次创作', '编辑图片', '转为视频']) expect(page.queryByRole('button', { name })).toBeNull();
});

test('Agent-created text has its own label and cannot be recalled as image creation', async () => {
  const textTask = task({ capability: 'text', status: 'succeeded', result_asset_ids: [assetId] });
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([textTask]);
  spyOn(creativeAssetClient, 'get').mockResolvedValue(asset('text'));
  const creation = composer();
  const page = renderTasks({ composer: creation });
  expect(await page.findByText('文本创作')).toBeTruthy();
  expect(await page.findByText('生成的文本')).toBeTruthy();
  expect(page.queryByText('图片生成')).toBeNull();
  expect(page.queryByRole('button', { name: '再次创作' })).toBeNull();
  expect(creationModeFor('text')).toBeNull();
  expect(() => recallCreationTask(creation.draft, textTask, [], 'image')).toThrow('此任务不属于');
});

test.each([
  ['i2i', '图片生成'], ['inpaint', '图片生成'], ['t2v', '视频生成'], ['i2v', '视频生成'], ['music', '音乐生成'], ['tts', '语音合成'],
] as [CreationTaskCapability, string][])('%s task shows its output kind without a composer', async (capability, label) => {
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task({ capability })]);
  const page = renderTasks();
  expect(await page.findByText(label)).toBeTruthy();
});

test.each(['queued', 'running'] as const)('%s images show an accessible placeholder driven by their actual task status', async status => {
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task({ status })]);
  const page = renderTasks();
  const label = status === 'queued' ? '排队中' : '生成中';
  const pending = await page.findByRole('group', { name: `图片生成：${label}` });
  expect(pending.getAttribute('aria-busy')).toBe('true');
  expect(pending.getAttribute('data-creation-pending')).toBe(status);
  expect(page.getByRole('status').getAttribute('aria-live')).toBe('polite');
  expect(page.getByText(status === 'queued' ? '等待生成图片' : '正在生成图片')).toBeTruthy();
  expect(page.queryByRole('progressbar')).toBeNull();
  expect(page.queryByRole('button', { name: '预览图片：产物' })).toBeNull();
});

test.each([
  ['t2v', '视频生成', '视频'], ['music', '音乐生成', '音乐'], ['tts', '语音合成', '语音'], ['text', '文本创作', '文本'],
] as [CreationTaskCapability, string, string][])('%s pending tasks describe the corresponding output', async (capability, title, output) => {
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task({ capability })]);
  const page = renderTasks();
  expect(await page.findByRole('group', { name: `${title}：生成中` })).toBeTruthy();
  expect(page.getByText(`正在生成${output}`)).toBeTruthy();
  expect(page.queryByRole('progressbar')).toBeNull();
});

test('queued and running placeholders are replaced by the completed image when task polling refreshes', async () => {
  const list = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task({ status: 'queued' })]);
  spyOn(creativeAssetClient, 'get').mockResolvedValue(asset());
  const page = renderTasks({ refreshControl: true });
  expect(await page.findByRole('group', { name: '图片生成：排队中' })).toBeTruthy();
  list.mockResolvedValue([task()]);
  fireEvent.click(page.getByRole('button', { name: '刷新测试任务' }));
  expect(await page.findByRole('group', { name: '图片生成：生成中' })).toBeTruthy();
  expect(page.queryByText('等待生成图片')).toBeNull();
  list.mockResolvedValue([task({ status: 'succeeded', result_asset_ids: [assetId] })]);
  fireEvent.click(page.getByRole('button', { name: '刷新测试任务' }));
  expect(await page.findByRole('button', { name: '预览图片：产物' })).toBeTruthy();
  expect(page.getByRole('status').textContent).toBe('已完成');
  expect(page.container.querySelector('[data-creation-pending]')).toBeNull();
  expect(page.container.querySelector('[aria-busy="true"]')).toBeNull();
  expect(page.queryByRole('button', { name: '取消任务' })).toBeNull();
});

test.each(['failed', 'canceled'] as const)('%s terminal tasks remove the running placeholder without showing a result', async status => {
  const list = spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task()]);
  const page = renderTasks({ refreshControl: true });
  expect(await page.findByRole('group', { name: '图片生成：生成中' })).toBeTruthy();
  list.mockResolvedValue([task({ status, error: status === 'failed' ? { message: '生成服务返回错误' } : null })]);
  fireEvent.click(page.getByRole('button', { name: '刷新测试任务' }));
  await waitFor(() => expect(page.getByRole('status').textContent).toBe(status === 'failed' ? '生成失败' : '已取消'));
  expect(page.container.querySelector('[data-creation-pending]')).toBeNull();
  expect(page.container.querySelector('[aria-busy="true"]')).toBeNull();
  expect(page.queryByRole('button', { name: '取消任务' })).toBeNull();
  expect(page.queryByRole('button', { name: '预览图片：产物' })).toBeNull();
  if (status === 'failed') {
    expect(page.getByRole('alert').textContent).toBe(conversation.agentError.codes.CONVERSATION_GENERATION_FAILED.title);
    expect(page.getByText('生成服务返回错误').closest('[hidden]')).not.toBeNull();
    fireEvent.click(page.getByRole('button', { name: conversation.agentError.expandDetails }));
    expect(page.getByText('生成服务返回错误').closest('[hidden]')).toBeNull();
    expect(page.getAllByText('image-model').some(element => element.tagName === 'DD')).toBe(true);
    expect(page.getByText(conversationId)).toBeTruthy();
    expect(page.getByText(messageId)).toBeTruthy();
  }
});

test('task status-read failures use shared guidance and retry without creating a Turn', async () => {
  const list = spyOn(creationClient, 'listCreationTasks').mockRejectedValue(new Error('Original status diagnostic'));
  const page = renderTasks();
  expect(await page.findByRole('alert')).toBeTruthy();
  expect(page.getByRole('alert').textContent).toBe(conversation.agentError.codes.CONVERSATION_CREATION_STATUS_FAILED.title);
  expect(page.getByText('Original status diagnostic').closest('[hidden]')).not.toBeNull();
  list.mockResolvedValue([]);
  fireEvent.click(page.getByRole('button', { name: common.retry }));
  await waitFor(() => expect(page.queryByRole('alert')).toBeNull());
  expect(page.container.querySelector('[data-creation-task]')).toBeNull();
});

test.each(['expired_key', 'insufficient_quota', 'spend_limit_reached'] as const)('generation failures retain captured %s diagnosis and actual provider context', async reason => {
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task({
    status: 'failed', finished_at: 1700000000000,
    error: { kind: 'provider_error', message: 'Safe generation failure detail', http_status: 429, providerDiagnostic: {
      reason, httpStatus: 429, providerId, modelName: 'actual-image-route', providerCode: reason,
      endpoint: 'https://images.example.test/v1/generate?api_key=sk-do-not-display',
      requestId: 'generation-request-42', transportDetail: 'Safe captured transport detail',
    } },
  })]);
  const page = renderTasks();
  expect((await page.findByRole('alert')).textContent).toBe(conversation.agentError.providerReasons[reason].title);
  expect(page.getByText(conversation.agentError.providerReasons[reason].body)).toBeTruthy();
  expect(page.getByText('Safe captured transport detail').closest('[hidden]')).not.toBeNull();
  expect(page.container.textContent).not.toContain('sk-do-not-display');
  fireEvent.click(page.getByRole('button', { name: conversation.agentError.expandDetails }));
  expect(page.getByText('Safe captured transport detail').closest('[hidden]')).toBeNull();
  expect(page.getByText('actual-image-route')).toBeTruthy();
  expect(page.getByText(providerId)).toBeTruthy();
  expect(page.getByText('https://images.example.test/v1/generate')).toBeTruthy();
  expect(page.getByText('generation-request-42')).toBeTruthy();
  const turn = page.getByText(conversation.agentError.details.turn).closest('div');
  expect(turn?.querySelector('dd')?.textContent).toBe(conversation.agentError.details.unavailable);
  expect(page.getByText(conversation.agentError.details.message).closest('div')?.querySelector('dd')?.textContent).toBe(messageId);
  expect(page.getByText(conversation.agentError.details.creationTask).closest('div')?.querySelector('dd')?.textContent).toBe('0190f5fe-7c00-7a00-8000-000000000108');
  expect(page.getByText(conversation.agentError.details.sessionModel)).toBeTruthy();
  expect(page.getByText('2023-11-14T22:13:20.000Z')).toBeTruthy();
  expect(page.queryByText('Safe generation failure detail')).toBeNull();
});

test('a failed creation task keeps its explicit adjustment action inside shared error controls', async () => {
  spyOn(creationClient, 'listCreationTasks').mockResolvedValue([task({ status: 'failed', error: null })]);
  const creation = composer();
  const page = renderTasks({ composer: creation });
  expect(await page.findByRole('alert')).toBeTruthy();
  const retry = page.getByRole('button', { name: testI18n.t('conversation.agentError.adjustAndRetry', { defaultValue: 'Adjust and retry' }) });
  expect(retry.closest('.message-error-note__controls')).not.toBeNull();
  fireEvent.click(retry);
  await waitFor(() => expect(creation.selectMode).toHaveBeenCalledWith('image'));
  expect(creation.update).toHaveBeenCalledTimes(1);
});

test('opens the shared full-screen image preview with redundant exit controls', async () => {
  const page = render(
    <I18nextProvider i18n={testI18n}>
      <ConversationImagePreview src='/cat.png' title='猫咪' />
    </I18nextProvider>
  );
  const trigger = page.getByRole('button', { name: '预览图片：猫咪' });
  expect(page.queryByRole('dialog', { name: '查看图片' })).toBeNull();

  fireEvent.click(trigger);
  const dialog = await page.findByRole('dialog', { name: '查看图片' });
  expect(dialog.className).toContain('nomifun-modal-fullscreen');
  const images = [...page.container.querySelectorAll('img'), ...document.body.querySelectorAll('.arco-modal-wrapper img')];
  expect(images).toHaveLength(2);
  expect(images[1].getAttribute('src')).toBe('/cat.png');
  expect(page.getByRole('button', { name: '关闭图片预览' })).toBeTruthy();

  fireEvent.click(page.getByRole('button', { name: '关闭图片预览' }));
  const wrapper = dialog.closest('.arco-modal-wrapper') as HTMLElement;
  await waitFor(() => expect(wrapper.isConnected).toBe(false));

  fireEvent.click(trigger);
  const escapeDialog = await page.findByRole('dialog', { name: '查看图片' });
  const focusLock = escapeDialog.querySelector('[data-focus-lock-disabled]') as HTMLElement;
  fireEvent.keyDown(focusLock, { key: 'Escape', code: 'Escape' });
  await waitFor(() => expect(escapeDialog.isConnected).toBe(false));

  fireEvent.click(trigger);
  const maskDialog = await page.findByRole('dialog', { name: '查看图片' });
  const previewImage = document.body.querySelector('.arco-modal-wrapper img') as HTMLImageElement;
  fireEvent.click(previewImage.parentElement!);
  await waitFor(() => expect(maskDialog.isConnected).toBe(false));
});
