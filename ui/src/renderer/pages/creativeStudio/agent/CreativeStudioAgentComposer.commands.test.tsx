import '../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState, type ComponentProps } from 'react';
import { I18nextProvider } from 'react-i18next';
import { SWRConfig } from 'swr';
import { Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { parseConversationId, parseMessageId, parseProviderId } from '@/common/types/ids';
import type { TChatConversation } from '@/common/config/storage';
import { configService } from '@/common/config/configService';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import CreativeStudioAgentComposer from './CreativeStudioAgentComposer';

const i18n = createInstance();
await i18n.init({ lng: 'en-US', resources: { 'en-US': { translation: { messages, conversation, common } } } });
const SESSION = '0190f5fe-7c00-7a00-8000-000000000311';
const model = { providerId: parseProviderId('0190f5fe-7c00-7a00-8000-000000000312'), model: 'chat-model' };
type Props = ComponentProps<typeof CreativeStudioAgentComposer>;
const idleSnapshot = () => ({ id: parseConversationId(SESSION), name: 'Canvas planning', type: 'nomi', status: 'finished' } as TChatConversation);
beforeEach(() => {
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(idleSnapshot());
  for (const event of [ipcBridge.conversation.listChanged, ipcBridge.conversation.turnStarted,
    ipcBridge.conversation.turnCompleted, ipcBridge.conversation.turnPaused, ipcBridge.conversation.reconnected]) {
    spyOn(event, 'on').mockImplementation(() => () => {});
  }
});
afterEach(() => { cleanup(); mock.restore(); });

function fixture(overrides: Partial<Props> = {}) {
  const send = mock();
  const stop = mock();
  const clear = spyOn(ipcBridge.conversation.clearContext, 'invoke').mockResolvedValue();
  const warning = spyOn(Message, 'warning').mockImplementation(() => () => {});
  spyOn(Message, 'success').mockImplementation(() => () => {});
  function Harness() {
    const [draft, setDraft] = useState(overrides.draft ?? '');
    return <CreativeStudioAgentComposer targetKey='canvas:draft' conversationId={SESSION} model={model}
      modelLocked={false} isRunning={false} disabled={false} contextItems={[
        { id: 'node-1', label: 'Current canvas node', type: 'text', selected: true },
      ]} skillOptions={[{ id: 'creative-studio-canvas', label: 'Canvas planning', description: '' }]}
      selectedSkillIds={['creative-studio-canvas']} onModelChange={() => {}} onRemoveContextItem={() => {}}
      onToggleSkill={() => {}} onSend={send} onStop={stop} {...overrides} draft={draft} onDraftChange={setDraft} />;
  }
  const page = render(<I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map() }}><Harness /></SWRConfig></I18nextProvider>);
  const input = page.getByRole('textbox') as HTMLTextAreaElement;
  return { page, input, send, stop, clear, warning,
    change: (value: string) => fireEvent.change(input, { target: { value } }),
    submit: () => fireEvent.click(page.getByRole('button', { name: 'Send to Agent' })),
  };
}

test.each(['button', 'keyboard', 'menu'])('Canvas dispatches clear through the canonical Session for %s', async gesture => {
  const f = fixture({ model: null, selectedSkillIds: [] });
  f.change('/clear');
  const option = await f.page.findByRole('option', { name: /\/clear/ });
  expect(f.page.queryByRole('option', { name: /\/open/ })).toBeNull();
  if (gesture === 'button') f.submit();
  else if (gesture === 'keyboard') fireEvent.keyDown(f.input, { key: 'Enter' });
  else fireEvent.click(option);
  await waitFor(() => expect(f.clear).toHaveBeenCalledTimes(1));
  expect(f.clear.mock.calls[0]![0].conversation_id).toBe(SESSION);
  await waitFor(() => expect(f.input.value).toBe(''));
  expect(f.send).not.toHaveBeenCalled();
  expect(f.stop).not.toHaveBeenCalled();
});

test('Canvas draft-only and invalid command submissions never become planning prompts', async () => {
  const f = fixture({ conversationId: undefined });
  f.change('/clear');
  expect((await f.page.findByRole('option', { name: /\/clear/ })).getAttribute('aria-disabled')).toBe('true');
  f.submit();
  expect(f.warning).toHaveBeenCalledTimes(1);
  expect(f.input.value).toBe('/clear');
  f.change('/clear unexpected arguments');
  fireEvent.keyDown(f.input, { key: 'Enter' });
  expect(f.warning).toHaveBeenCalledTimes(2);
  expect(f.clear).not.toHaveBeenCalled();
  expect(f.send).not.toHaveBeenCalled();
});

test('Canvas refuses context clear while a Turn is running and retains its stop action for normal text', () => {
  const f = fixture({ isRunning: true });
  f.change('/clear');
  expect(f.page.getByRole('button', { name: 'Stop Agent' })).toBeTruthy();
  f.submit();
  expect(f.warning).toHaveBeenCalledTimes(1);
  expect(f.clear).not.toHaveBeenCalled();
  expect(f.send).not.toHaveBeenCalled();
  f.change('Continue planning');
  fireEvent.click(f.page.getByRole('button', { name: 'Stop Agent' }));
  expect(f.stop).toHaveBeenCalledTimes(1);
});

test('Canvas preserves explicit context and Skills for ordinary prompts and unknown slash text', async () => {
  const f = fixture();
  await act(async () => {});
  f.change('/a-script-path');
  f.submit();
  expect(f.send).toHaveBeenCalledWith({ prompt: '/a-script-path', model,
    contextNodeIds: ['node-1'], skillIds: ['creative-studio-canvas'] });
  expect(f.clear).not.toHaveBeenCalled();
});

test('Canvas does not dispatch a command while IME is composing or in its composition-end frame', async () => {
  const f = fixture();
  f.change('/clear');
  fireEvent.compositionStart(f.input);
  fireEvent.keyDown(f.input, { key: 'Enter' });
  fireEvent.compositionEnd(f.input);
  fireEvent.keyDown(f.input, { key: 'Enter' });
  await act(async () => {});
  expect(f.clear).not.toHaveBeenCalled();
  expect(f.send).not.toHaveBeenCalled();
});

test('Canvas follows the shared mod-enter preference for ordinary prompts and complete commands', async () => {
  spyOn(configService, 'get').mockReturnValue('mod-enter');
  const f = fixture();
  f.change('Continue the canvas plan');
  fireEvent.keyDown(f.input, { key: 'Enter' });
  expect(f.send).not.toHaveBeenCalled();
  fireEvent.keyDown(f.input, { key: 'Enter', ctrlKey: true });
  expect(f.send).toHaveBeenCalledTimes(1);
  f.change('/clear');
  await waitFor(async () => expect((await f.page.findByRole('option', { name: /\/clear/ })).getAttribute('aria-disabled')).toBe('false'));
  f.change('/clear ');
  fireEvent.keyDown(f.input, { key: 'Enter' });
  expect(f.clear).not.toHaveBeenCalled();
  fireEvent.keyDown(f.input, { key: 'Enter', metaKey: true });
  await waitFor(() => expect(f.clear).toHaveBeenCalledTimes(1));
  expect(f.send).toHaveBeenCalledTimes(1);
});

test('Canvas never treats an unhydrated Session as idle and enables clear only after its canonical snapshot', async () => {
  let resolve!: (snapshot: TChatConversation) => void;
  const read = spyOn(ipcBridge.conversation.get, 'invoke').mockImplementationOnce(() => new Promise(done => { resolve = done; }));
  const f = fixture();
  f.change('/clear');
  const option = await f.page.findByRole('option', { name: /\/clear/ });
  expect(option.getAttribute('aria-disabled')).toBe('true');
  expect(option.title).toBe(messages.commands.stateUnconfirmed);
  f.submit();
  expect(f.clear).not.toHaveBeenCalled();
  await act(async () => { resolve(idleSnapshot()); });
  await waitFor(() => expect(option.getAttribute('aria-disabled')).toBe('false'));
  f.submit();
  await waitFor(() => expect(f.clear).toHaveBeenCalledTimes(1));
  expect(read.mock.calls[0]![0].conversation_id).toBe(SESSION);
  expect(f.send).not.toHaveBeenCalled();
});

test('Canvas blocks clear for an authoritative pause even when its local controller is stopped, then follows fresh idle', async () => {
  const read = spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({
    ...idleSnapshot(), status: 'running', extra: { workspace: '/canvas', execution_phase: 'paused',
      execution_pause: { reason: 'EXECUTION_MODEL_PROVIDER_UNAVAILABLE', cleanup_proven: true, paused_at_ms: 123 } },
    runtime: { state: 'idle', is_processing: false, can_send_message: false,
      active_turn_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000314') },
  } as TChatConversation);
  const f = fixture({ isRunning: false });
  f.change('/clear');
  const option = await f.page.findByRole('option', { name: /\/clear/ });
  await waitFor(() => expect(option.title).toBe(messages.commands.clearAfterTurn));
  expect(option.getAttribute('aria-disabled')).toBe('true');
  fireEvent.click(option);
  f.submit();
  expect(f.clear).not.toHaveBeenCalled();
  read.mockResolvedValue(idleSnapshot());
  const refresh = spyOn(ipcBridge.conversation.listChanged, 'on').mock.calls[0]![0];
  act(() => { refresh({ conversation_id: parseConversationId(SESSION), action: 'updated' }); });
  await waitFor(() => expect(option.getAttribute('aria-disabled')).toBe('false'));
  f.submit();
  await waitFor(() => expect(f.clear).toHaveBeenCalledTimes(1));
  expect(f.send).not.toHaveBeenCalled();
});

test.each(['unknown', 'error'])('Canvas refuses clear when canonical authority is %s', async state => {
  const read = spyOn(ipcBridge.conversation.get, 'invoke');
  if (state === 'error') read.mockRejectedValue(new Error('State unavailable'));
  else read.mockResolvedValue({ ...idleSnapshot(), status: 'running', runtime: undefined });
  const f = fixture();
  f.change('/clear');
  const option = await f.page.findByRole('option', { name: /\/clear/ });
  await act(async () => {});
  expect(option.getAttribute('aria-disabled')).toBe('true');
  expect(option.title).toBe(messages.commands.stateUnconfirmed);
  f.submit();
  expect(f.clear).not.toHaveBeenCalled();
  expect(f.send).not.toHaveBeenCalled();
});

test('Canvas export keeps its shared follow-up panel open through filename entry and saves canonical history', async () => {
  const id = parseConversationId(SESSION);
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id, name: 'Canvas planning', type: 'nomi' } as TChatConversation);
  spyOn(ipcBridge.application.getPath, 'invoke').mockResolvedValue('/exports');
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({
    items: [{ id: 'original', message_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000313'),
      conversation_id: id, type: 'text', position: 'right', content: { content: 'Canonical planning request' }, created_at: 1 }],
    total: 1, has_more: false,
  });
  const write = spyOn(ipcBridge.fs.writeFile, 'invoke').mockResolvedValue(true);
  const f = fixture();
  f.change('/export');
  f.submit();
  fireEvent.click(await f.page.findByRole('option', { name: /Save to file/ }));
  const filename = f.page.getAllByRole('textbox').find(element => element.tagName === 'INPUT')!;
  fireEvent.change(filename, { target: { value: 'planning.txt' } });
  fireEvent.keyDown(filename, { key: 'Enter' });
  await waitFor(() => expect(write).toHaveBeenCalledTimes(1));
  expect(write.mock.calls[0]![0].path).toBe('/exports/planning.txt');
  expect(write.mock.calls[0]![0].data).toContain('Canonical planning request');
  expect(history.mock.calls[0]![0].conversation_id).toBe(SESSION);
  await waitFor(() => expect(f.page.queryByRole('listbox')).toBeNull());
  expect(f.send).not.toHaveBeenCalled();
  expect(f.clear).not.toHaveBeenCalled();
});
