import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import { createStoredMessageMapper } from '@/common/adapter/storedMessageMapper';
import type { IResponseMessage, IConversationTurnCompletedEvent } from '@/common/adapter/ipcBridge';
import type { TChatConversation } from '@/common/config/storage';
import type { TMessage } from '@/common/chat/chatLib';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { ThemeProvider } from '@/renderer/hooks/context/ThemeContext';
import { PreviewProvider } from '../Preview';
import { useNomiMessage } from '../platforms/nomi/useNomiMessage';
import MessageList from './MessageList';
import { MessageListLoadingProvider, MessageListProvider, useMessageLstCache } from './hooks';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages, conversation } } } });
const id = (n: number) => parseMessageId(`0190f5fe-7c00-7a00-8000-${String(n).padStart(12, '0')}`);
const question = { request_id: 'ask:profile', question: '请提供姓名、学校和喜欢的风格。', remaining_work: '收到信息后完成五页演示文稿。' };
const scenarios = [
  { session: '01a13a3f-769c-7ae2-9083-a41a6e91d6bf', root: '01a13a46-0592-7f71-a82f-4987ea7ae7b2',
    summary: '01a13a46-0592-7f71-a82f-4995148bb124', empty: '01a13a46-0592-7f71-a82f-4987ea7ae7b3', userSeq: 146, startSeq: 147, textSeq: 182 },
  { session: '01a13e31-57e4-74b1-8f38-19a49e07210c', root: '01a13e31-5dea-70f3-9cb5-32231d832a02',
    summary: '01a13e31-5dea-70f3-9cb5-323f5b5ee88f', empty: '01a13e31-5dea-70f3-9cb5-32231d832a03', userSeq: 4, startSeq: 5, textSeq: 85 },
];

beforeEach(() => {
  spyOn(configService, 'get').mockImplementation(() => undefined);
  spyOn(configService, 'subscribe').mockImplementation(() => () => {});
  spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockReturnValue(() => {});
  for (const name of ['turnStarted', 'turnPaused', 'userCreated', 'messageAnnotated', 'reconnected'] as const) {
    spyOn(ipcBridge.conversation[name], 'on').mockReturnValue(() => {});
  }
  spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(undefined);
  spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockReturnValue(() => {});
  spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
  spyOn(ipcBridge.conversation.turnFiles, 'invoke').mockResolvedValue([]);
});
afterEach(() => { cleanup(); mock.restore(); });

function fixture(scenario: typeof scenarios[number]) {
  const session = parseConversationId(scenario.session);
  const root = parseMessageId(scenario.root);
  const summary = parseMessageId(scenario.summary);
  const empty = parseMessageId(scenario.empty);
  let rowKey = 0;
  const map = createStoredMessageMapper(() => `canonical-${++rowKey}`);
  const row = (messageId: ReturnType<typeof id>, type: TMessage['type'], seq: number, content: object, position = 'left', turnId: typeof root | undefined = root) => map({
    message_id: messageId, msg_id: messageId, conversation_id: session, type, position, hidden: false,
    created_at: 100_000 + seq, content: { ...content, ...(turnId ? { turn_id: turnId } : {}) },
  });
  const user = row(root, 'text', scenario.userSeq, { content: '设计开学自我介绍，三到五页即可。' }, 'right');
  const handoff = row(summary, 'tips', scenario.startSeq, { type: 'warning', content: '', user_input_request: question,
    turn_outcome: { kind: 'input_required', request_id: question.request_id }, started_at_ms: 1000, finished_at_ms: 38000 }, 'center');
  const thought = row(id(801), 'thinking', scenario.startSeq + 10, { content: 'Determine the required information.', status: 'done' });
  const preamble = row(id(802), 'text', scenario.startSeq + 12, { content: '我先整理演示文稿结构。' });
  // Mirrors both incident databases. The new host omits this synthetic row,
  // and a delayed/cached empty row still cannot elect a public result.
  const emptyText = row(empty, 'text', scenario.textSeq, { content: '' }, 'left', undefined);
  return { session, root, summary, empty, row, user, handoff, thought, preamble, emptyText };
}

function setup(f: ReturnType<typeof fixture>, initial: TMessage[], readOnly = false) {
  let historyRows = initial;
  let historyCalls = 0;
  const completionListeners = new Set<(event: IConversationTurnCompletedEvent) => void>();
  let stream: (message: IResponseMessage) => void = () => {};
  spyOn(ipcBridge.conversation.responseStream, 'on').mockImplementation(listener => { stream = listener; return () => {}; });
  spyOn(ipcBridge.conversation.turnCompleted, 'on').mockImplementation(listener => {
    completionListeners.add(listener); return () => completionListeners.delete(listener);
  });
  const idle = { state: 'idle', can_send_message: true, has_runtime: false, is_processing: false } as const;
  const authority = { id: f.session, type: 'nomi', status: 'finished', extra: { workspace: '' }, runtime: idle } as TChatConversation;
  spyOn(ipcBridge.conversation.get, 'invoke').mockResolvedValue(authority);
  spyOn(ipcBridge.database.getConversationMessages, 'invoke').mockImplementation(async () => {
    historyCalls++; return { items: [...historyRows].reverse(), total: historyRows.length, has_more: false };
  });
  const Timeline = () => {
    const activity = useNomiMessage(f.session);
    const paging = useMessageLstCache(f.session);
    return <ConversationProvider value={{ conversation_id: f.session, type: 'nomi', readOnly,
      isProcessing: activity.running, isTurnStateHydrated: activity.hasHydratedRunningState }}>
      <MessageList hasMoreOlder={paging.hasMore} onLoadOlder={paging.loadOlder} />
    </ConversationProvider>;
  };
  const App = ({ visit = 0 }: { visit?: number }) => <MemoryRouter><I18nextProvider i18n={i18n}><ThemeProvider>
    <PreviewProvider persistNamespace={`turn-outcome-${f.session}`} subscribeGlobalOpen={false}>
      <MessageListProvider key={visit}><MessageListLoadingProvider><Timeline /></MessageListLoadingProvider></MessageListProvider>
    </PreviewProvider>
  </ThemeProvider></I18nextProvider></MemoryRouter>;
  return { App, stream: (message: IResponseMessage) => stream(message), historyCalls: () => historyCalls,
    setHistory: (rows: TMessage[]) => { historyRows = rows; },
    complete: () => { for (const listener of completionListeners) listener({ conversation_id: f.session, turn_id: f.root, runtime: idle } as IConversationTurnCompletedEvent); },
  };
}

const markdownText = (node: Element) => Array.from(node.querySelectorAll('.markdown-shadow'), child => child.shadowRoot?.textContent ?? '').join('\n');
const assertPublicQuestion = async (page: ReturnType<typeof render>) => {
  const note = await page.findByTestId('user-input-request-note');
  expect(note.closest('.turn-process-disclosure')).toBeNull();
  await waitFor(() => expect(markdownText(note)).toContain(question.question));
  expect(markdownText(note)).toContain(question.remaining_work);
  expect(page.getAllByTestId('user-input-request-note')).toHaveLength(1);
};

test.each(scenarios)('canonical handoff survives completion refresh, expansion and reopening: $session', async scenario => {
  const f = fixture(scenario);
  const h = setup(f, [f.user]);
  const page = render(<h.App />);
  await waitFor(() => expect(h.historyCalls()).toBe(1));
  await act(async () => h.stream({ conversation_id: f.session, turn_id: f.root, msg_id: f.summary,
    type: 'tips', created_at: 200_000, data: f.handoff.content }));
  await assertPublicQuestion(page);
  h.setHistory([f.user, f.handoff, f.thought, f.preamble, f.emptyText]);
  await act(async () => h.complete());
  await waitFor(() => expect(h.historyCalls()).toBeGreaterThan(1));
  await assertPublicQuestion(page);
  const toggle = page.container.querySelector<HTMLButtonElement>('.turn-process-disclosure__toggle')!;
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(toggle);
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  await assertPublicQuestion(page);
  expect(markdownText(page.container.querySelector('.turn-process-disclosure')!)).toContain('我先整理演示文稿结构。');
  fireEvent.click(toggle);
  await assertPublicQuestion(page);
  page.rerender(<h.App visit={1} />);
  await assertPublicQuestion(page);
});

test('readonly handoff remains public beside a later Turn and late old output cannot displace either result', async () => {
  const f = fixture(scenarios[0]);
  const nextRoot = id(820);
  const nextText = f.row(id(821), 'text', 202, { content: '后续回合的实际回答。' }, 'left', nextRoot);
  const rows = [f.user, f.handoff, f.thought, f.emptyText,
    f.row(nextRoot, 'text', 200, { content: '请先用通用校园风格。' }, 'right', nextRoot),
    f.row(id(822), 'agent_status', 201, { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed',
      started_at_ms: 40000, finished_at_ms: 42000, turn_outcome: { kind: 'answered', message_ids: [id(821)] } }, 'center', nextRoot), nextText];
  const h = setup(f, rows, true);
  const page = render(<h.App />);
  await assertPublicQuestion(page);
  expect(page.queryByRole('button', { name: conversation.userInputRequest.addInput })).toBeNull();
  await act(async () => h.stream({ conversation_id: f.session, turn_id: f.root, msg_id: id(823), type: 'content',
    created_at: 999999, data: { content: '迟到的前一回合过程文字。' } }));
  await assertPublicQuestion(page);
  await waitFor(() => expect(markdownText(page.container)).toContain('后续回合的实际回答。'));
  expect(markdownText(page.container)).not.toContain('迟到的前一回合过程文字。');
});

test('answered membership ignores later empty/narration rows and unavailable history has an explicit public notice', async () => {
  const f = fixture(scenarios[1]);
  const answer = f.row(id(830), 'text', 40, { content: '这是已提交的最终回答。' });
  const summary = { ...f.handoff, type: 'agent_status' as const, content: { backend: 'nomi', status: 'prepared' as const,
    turn_summary: true, turn_state: 'completed' as const, started_at_ms: 1000, finished_at_ms: 38000,
    turn_outcome: { kind: 'answered' as const, message_ids: [id(830)] } } };
  const h = setup(f, [f.user, summary, f.preamble, answer, f.emptyText]);
  const page = render(<h.App />);
  await waitFor(() => expect(markdownText(page.container)).toContain('这是已提交的最终回答。'));
  expect(markdownText(page.container)).not.toContain('我先整理演示文稿结构。');
  h.setHistory([f.user, { ...summary, content: { ...summary.content, turn_outcome: { kind: 'unavailable' } } }]);
  // A new visit has no stale cached answer that could stand in for this receipt.
  page.rerender(<h.App visit={1} />);
  const notice = await page.findByTestId('turn-outcome-unavailable');
  expect(notice.closest('.turn-process-disclosure')).toBeNull();
  expect(notice.textContent).toContain(messages.turnOutcomeUnavailable);
});

test('silent robot action keeps a settled tool trace through live completion, refresh and cold reopen', async () => {
  const f = fixture(scenarios[0]);
  const toolId = id(870);
  const tool = f.row(toolId, 'tool_call', scenarios[0].startSeq + 20, { call_id: 'robot-sit', name: 'robot_action',
    status: 'completed', description: '坐下动作' });
  const receipt = f.row(f.summary, 'agent_status', scenarios[0].startSeq + 21, { backend: 'nomi', status: 'prepared',
    turn_summary: true, turn_state: 'completed', started_at_ms: 1000, finished_at_ms: 3000, turn_outcome: { kind: 'silent' } }, 'center');
  const h = setup(f, [f.user]);
  const page = render(<h.App />);
  await waitFor(() => expect(h.historyCalls()).toBe(1));
  await act(async () => h.stream({ conversation_id: f.session, turn_id: f.root, msg_id: toolId,
    type: 'tool_call', created_at: tool.created_at, data: tool.content }));
  await act(async () => h.stream({ conversation_id: f.session, turn_id: f.root, msg_id: f.summary,
    type: 'agent_status', stream_complete: true, data: receipt.content }));
  const assertSilent = async () => {
    await waitFor(() => expect(page.container.querySelector('.turn-process-disclosure__label')?.textContent).toBe('Worked for 2s'));
    expect(page.container.querySelector('.turn-process-disclosure--live')).toBeNull();
    expect(page.queryByTestId('turn-outcome-unavailable')).toBeNull();
    expect(page.queryByTestId('turn-outcome-read-error')).toBeNull();
    expect(page.container.querySelectorAll('[data-message-position="left"]')).toHaveLength(0);
    expect(page.container.querySelector(`[data-message-business-id="${f.summary}"]`)).toBeNull();
    const toggle = page.container.querySelector<HTMLButtonElement>('.turn-process-disclosure__toggle')!;
    expect(toggle.getAttribute('aria-expanded')).toBe('false');
    fireEvent.click(toggle);
    expect(page.container.querySelector('[data-tool-call-id="robot-sit"]')).not.toBeNull();
    fireEvent.click(toggle);
  };
  await assertSilent();
  h.setHistory([f.user, tool, receipt]);
  await act(async () => h.complete());
  await waitFor(() => expect(h.historyCalls()).toBeGreaterThan(1));
  await assertSilent();
  page.rerender(<h.App visit={1} />);
  await assertSilent();
});

test('a cold silent Turn does not suppress a following ordinary answer or failed action', async () => {
  const f = fixture(scenarios[1]);
  const nextRoot = id(880);
  const answerId = id(881);
  const failedRoot = id(882);
  const failureId = id(883);
  const rows = [f.user,
    f.row(f.summary, 'agent_status', 31, { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed',
      started_at_ms: 1000, finished_at_ms: 3000, turn_outcome: { kind: 'silent' } }, 'center'),
    f.row(nextRoot, 'text', 40, { content: '北京今天的天气怎么样？' }, 'right', nextRoot),
    f.row(id(884), 'agent_status', 41, { backend: 'nomi', status: 'prepared', turn_summary: true, turn_state: 'completed',
      started_at_ms: 4000, finished_at_ms: 5000, turn_outcome: { kind: 'answered', message_ids: [answerId] } }, 'center', nextRoot),
    f.row(answerId, 'text', 42, { content: '北京今天晴。' }, 'left', nextRoot),
    f.row(failedRoot, 'text', 50, { content: '站起来。' }, 'right', failedRoot),
    f.row(failureId, 'tips', 51, { type: 'error', content: '机器人连接已断开。',
      error: { message: '机器人连接已断开。', code: 'ROBOT_DISCONNECTED', retryable: false },
      turn_outcome: { kind: 'failed', message_ids: [failureId] } }, 'center', failedRoot)];
  const h = setup(f, rows);
  const page = render(<h.App />);
  await waitFor(() => expect(markdownText(page.container)).toContain('北京今天晴。'));
  await waitFor(() => expect(page.container.textContent).toContain('机器人连接已断开。'));
  expect(page.queryByTestId('turn-outcome-unavailable')).toBeNull();
  expect(page.queryByTestId('user-input-request-note')).toBeNull();
});

test('a result-read failure is a public notice and never replaces an existing canonical outcome', async () => {
  const f = fixture(scenarios[0]);
  const h = setup(f, [f.user, f.handoff]);
  const page = render(<h.App />);
  await assertPublicQuestion(page);
  await act(async () => h.stream({ conversation_id: f.session, turn_id: f.root, msg_id: id(850), type: 'tips',
    stream_complete: true, data: { type: 'error', query_error: true, content: 'Canonical result read failed' } }));
  const notice = await page.findByTestId('turn-outcome-read-error');
  expect(notice.closest('.turn-process-disclosure')).toBeNull();
  expect(notice.textContent).toContain(messages.turnOutcomeReadFailed);
  await assertPublicQuestion(page);
});

test('an explicit standalone synthesis remains public after an older canonical Turn and after reopening', async () => {
  const f = fixture(scenarios[1]);
  const replyId = id(860);
  const synthesisId = id(861);
  const answer = f.row(replyId, 'text', 40, { content: '原回合已提交的回答。' });
  const receipt: TMessage = { ...f.handoff, type: 'agent_status', content: { backend: 'nomi', status: 'prepared',
    turn_summary: true, turn_state: 'completed', started_at_ms: 1000, finished_at_ms: 38000,
    turn_outcome: { kind: 'answered', message_ids: [replyId] } } };
  const h = setup(f, [f.user, receipt, answer]);
  const page = render(<h.App />);
  await waitFor(() => expect(markdownText(page.container)).toContain('原回合已提交的回答。'));
  await act(async () => h.stream({ conversation_id: f.session, msg_id: synthesisId, type: 'content',
    stream_complete: true, data: { content: '独立协作任务的合成回复。' } }));
  await waitFor(() => expect(markdownText(page.container)).toContain('独立协作任务的合成回复。'));
  const assertStandalone = () => {
    const row = page.container.querySelector(`[data-message-business-id="${synthesisId}"]`)!;
    expect(row).not.toBeNull();
    expect(row.closest('.turn-process-disclosure')).toBeNull();
    expect(page.container.querySelectorAll('.turn-process-disclosure')).toHaveLength(1);
    expect(page.container.querySelector('.turn-process-disclosure--live')).toBeNull();
  };
  assertStandalone();
  // The host authors this marker from message/assistant-projected. A missing
  // turn_id alone does not make any historical text a standalone projection.
  const synthesis: TMessage = { ...f.row(synthesisId, 'text', 1000,
    { content: '独立协作任务的合成回复。', standalone_projection: true }), turn_id: undefined };
  const unmarked: TMessage = { ...f.row(id(862), 'text', 1001,
    { content: '没有明确独立展示来源的文字。' }), turn_id: undefined };
  h.setHistory([f.user, receipt, answer, synthesis, unmarked]);
  page.rerender(<h.App visit={1} />);
  await waitFor(() => expect(markdownText(page.container)).toContain('独立协作任务的合成回复。'));
  expect(markdownText(page.container)).toContain('原回合已提交的回答。');
  expect(page.container.querySelector(`[data-message-business-id="${id(862)}"]`)).toBeNull();
  assertStandalone();
});
