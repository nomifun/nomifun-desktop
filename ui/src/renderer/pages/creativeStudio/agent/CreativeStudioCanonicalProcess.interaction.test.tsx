import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseExecutionAttemptId, parseMessageId } from '@/common/types/ids';
import { configService } from '@/common/config/configService';
import { createStoredMessageMapper } from '@/common/adapter/storedMessageMapper';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import CreativeStudioAgentMessages from './CreativeStudioAgentMessages';
import { CreativeStudioCanonicalProcessProvider } from './CreativeStudioCanonicalProcess';
import { makeDelegationRecord, makeStep } from '../../../../../test/fixtures/conversationDelegation';
import { useMessageList } from '@/renderer/pages/conversation/Messages/hooks';

afterEach(() => { cleanup(); mock.restore(); });

test.each([false, true])('Canvas reopens one canonical process with batched delegation %j and fetches exact tool details only when expanded', async withDelegation => {
  const session = parseConversationId('0190f5fe-7c00-7a00-8000-000000000701');
  const root = parseMessageId('0190f5fe-7c00-7a00-8000-000000000702');
  const answer = parseMessageId('0190f5fe-7c00-7a00-8000-000000000703');
  const second = parseMessageId('0190f5fe-7c00-7a00-8000-000000000704');
  const tool = parseMessageId('0190f5fe-7c00-7a00-8000-000000000705');
  const summary = parseMessageId('0190f5fe-7c00-7a00-8000-000000000706');
  const instance = createInstance();
  await instance.use(initReactI18next).init({ lng: 'en-US', fallbackLng: 'en-US', resources: { 'en-US': { translation: { messages } } }, interpolation: { escapeValue: false } });
  const mapper = createStoredMessageMapper(() => crypto.randomUUID());
  const row = (id: typeof root, type: TMessage['type'], content: object, position = 'left') => mapper({
    message_id: id, msg_id: id, conversation_id: session, type, position, hidden: false, created_at: 100,
    ...(type === 'tool_call' ? { status: 'finish' } : {}),
    content: { ...content, turn_id: root },
  });
  const canonical = [
    row(root, 'text', { content: '{"secret_planning_input":"never show"}' }, 'right'),
    row(tool, 'tool_call', { call_id: 'canvas-command', name: 'start_process', capability_id: 'workspace.process', action_id: 'workspace.process/start',
      description: 'bun test', status: 'completed', result_seq: 12,
      timeline: { through_seq: 20, segments: [{ segment_id: 'call-event', group_id: `process:${root}`, first_seq: 10, last_seq: 12, kind: 'tool', state: 'completed', call_id: 'canvas-command' }] } }),
    row(parseMessageId('0190f5fe-7c00-7a00-8000-000000000707'), 'tool_call', {
      call_id: 'canvas-report', name: 'report_completion', description: 'Canvas result', status: 'completed', result_seq: 18 }),
    row(answer, 'text', { content: 'Canvas result' }), row(second, 'text', { content: 'Second result' }),
    row(summary, 'agent_status', { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed', started_at_ms: 1000, finished_at_ms: 2000 }),
  ];
  spyOn(configService, 'get').mockImplementation(() => undefined);
  spyOn(configService, 'subscribe').mockReturnValue(() => {});
  for (const name of ['responseStream', 'turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated', 'reconnected', 'listChanged'] as const)
    spyOn(ipcBridge.conversation[name], 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.agentExecution.events.changed, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id: session, type: 'nomi', status: 'finished', extra: { workspace: '/tmp/canvas' },
    runtime: { state: 'idle', is_processing: false, can_send_message: true } } as TChatConversation);
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [...canonical].reverse(), total: canonical.length, has_more: false });
  const original = makeDelegationRecord();
  const starts = Array.from({ length: 5 }, (_, index) => makeDelegationRecord({ id: 'canvas-start-' + index,
    title: 'Canvas task ' + index, step_id: makeStep(index).step_id, execution_event_sequence: index + 1,
    source: { ...original.source, agent_session_id: session, source_message_id: root, tool_event_seq: 4 },
    anchor: { ...original.anchor, agent_session_id: session, seq: 5 } }));
  const completions = starts.slice(0, 2).map((record, index) => ({ ...record, id: 'canvas-end-' + index, kind: 'completed' as const,
    execution_event_sequence: index + 10, attempt_id: parseExecutionAttemptId('00000000-09c4-7000-8000-' + String(30 + index).padStart(12, '0')),
    anchor: { ...record.anchor, seq: 15 } }));
  spyOn(ipcBridge.database.getDelegationRecords, 'invoke').mockResolvedValue({ records: withDelegation ? [...starts, ...completions] : [], through_session_seq: 20 });
  const details = spyOn(ipcBridge.database.getToolDetails, 'invoke').mockImplementation(async request => ({
    turn_id: root, call_id: 'canvas-command', call_event_id: 'call-event', through_seq: 20, result_seq: 12,
    section: request.section, availability: 'available', format: request.section === 'arguments' ? 'json' : 'text',
    content: request.section === 'arguments' ? '{"cmd":"bun test"}' : 'verified output', offset: 0,
    total_bytes: request.section === 'arguments' ? 18 : 15, eof: true, non_text_parts: 0,
  }));
  const View = () => <I18nextProvider i18n={instance}><CreativeStudioCanonicalProcessProvider conversationId={session}>
    <CreativeStudioAgentMessages conversationId={session} messages={[
      { id: answer, role: 'assistant', status: 'complete', text: 'Canvas result' },
      { id: second, role: 'assistant', status: 'complete', text: 'Second result' },
    ]} proposals={[]} proposalApplyDisabled={false} onApplyProposal={() => {}} />
  </CreativeStudioCanonicalProcessProvider></I18nextProvider>;
  for (let visit = 0; visit < 2; visit++) {
    const page = render(<View />);
    await waitFor(() => expect(page.getAllByRole('button', { name: instance.t('messages.turnProcess.expand') })).toHaveLength(1));
    expect(details.mock.calls).toHaveLength(visit * 2);
    expect(page.container.textContent).not.toContain('secret_planning_input');
    fireEvent.click(page.getByRole('button', { name: instance.t('messages.turnProcess.expand') }));
    expect(Boolean(page.container.querySelector('[data-tool-call-id="canvas-report"]'))).toBe(false);
    fireEvent.click(await page.findByRole('button', { name: /bun test/ }));
    await waitFor(() => expect(page.container.textContent).toContain('verified output'));
    expect(details.mock.calls).toHaveLength((visit + 1) * 2);
    if (withDelegation) {
      await waitFor(() => expect(page.container.querySelectorAll('[data-delegation-record-count]')).toHaveLength(2));
      expect([...page.container.querySelectorAll('[data-delegation-record-count]')].map(node => node.getAttribute('data-delegation-record-count'))).toEqual(['5', '2']);
      expect(page.container.querySelectorAll('[data-delegation-record-id]')).toHaveLength(0);
      fireEvent.click(page.getByRole('button', { name: instance.t('messages.delegation.toggleRecords', {
        action: instance.t('messages.delegation.action.delegated'), count: 5 }) }));
      expect(page.container.querySelectorAll('[data-delegation-record-id]')).toHaveLength(5);
      expect(page.container.querySelector('[data-tool-call-id="canvas-command"]')).not.toBeNull();
    }
    for (const [request] of details.mock.calls) {
      expect(request.conversation_id).toBe(session);expect(request.turn_id).toBe(root);
      expect(request.call_id).toBe('canvas-command');expect(request.expected_result_seq).toBe(12);
    }
    page.unmount();
  }
});

test('Canvas with only a confirmed completion receipt keeps its public answer without an empty process', async () => {
  const session = parseConversationId('0190f5fe-7c00-7a00-8000-000000000721');
  const root = parseMessageId('0190f5fe-7c00-7a00-8000-000000000722');
  const answer = parseMessageId('0190f5fe-7c00-7a00-8000-000000000723');
  const instance = createInstance();
  await instance.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages } } } });
  const mapper = createStoredMessageMapper(() => crypto.randomUUID());
  const row = (id: string, type: TMessage['type'], content: object, position = 'left') => mapper({
    message_id: id, msg_id: id, conversation_id: session, type, position, hidden: false, created_at: 100,
    ...(type === 'tool_call' ? { status: 'finish' } : {}),
    content: { ...content, turn_id: root },
  });
  const text = 'The report_completion result is delivered once.';
  const canonical = [row(root, 'text', { content: 'Complete the requested work.' }, 'right'),
    row('0190f5fe-7c00-7a00-8000-000000000724', 'tool_call', {
      name: 'report_completion', call_id: 'only-report', status: 'completed', result_seq: 12, description: text }),
    row(answer, 'text', { content: text }),
    row('0190f5fe-7c00-7a00-8000-000000000725', 'agent_status', { backend: 'nomi', status: 'prepared', turn_summary: true,
      turn_state: 'completed', started_at_ms: 1000, finished_at_ms: 2000 })];
  spyOn(configService, 'get').mockImplementation(() => undefined);
  spyOn(configService, 'subscribe').mockReturnValue(() => {});
  for (const name of ['responseStream', 'turnStarted', 'turnCompleted', 'turnPaused', 'userCreated', 'messageAnnotated', 'reconnected', 'listChanged'] as const)
    spyOn(ipcBridge.conversation[name], 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.agentExecution.events.changed, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue({ id: session, type: 'nomi', status: 'finished', extra: {},
    runtime: { state: 'idle', is_processing: false, can_send_message: true } } as TChatConversation);
  const history = spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockResolvedValue({ items: [...canonical].reverse(), total: canonical.length, has_more: false });
  spyOn(ipcBridge.database.getDelegationRecords, 'invoke').mockResolvedValue({ records: [], through_session_seq: 20 });
  const details = spyOn(ipcBridge.database.getToolDetails, 'invoke').mockRejectedValue(new Error('Completion payload must not be requested'));
  const Loaded = () => <output data-testid='loaded-canonical-count'>{useMessageList().length}</output>;
  const page = render(<I18nextProvider i18n={instance}><CreativeStudioCanonicalProcessProvider conversationId={session}>
    <Loaded />
    <CreativeStudioAgentMessages conversationId={session} messages={[{ id: answer, role: 'assistant', status: 'complete', text }]}
      proposals={[]} proposalApplyDisabled={false} onApplyProposal={() => {}} />
  </CreativeStudioCanonicalProcessProvider></I18nextProvider>);
  await waitFor(() => expect(history).toHaveBeenCalled());
  await waitFor(() => expect(page.getByTestId('loaded-canonical-count').textContent).toBe(String(canonical.length)));
  await waitFor(() => expect(page.getAllByText(text)).toHaveLength(1));
  expect(Boolean(page.container.querySelector('.turn-process-disclosure'))).toBe(false);
  expect(Boolean(page.container.querySelector('[data-tool-call-id="only-report"]'))).toBe(false);
  expect(details).not.toHaveBeenCalled();
});
