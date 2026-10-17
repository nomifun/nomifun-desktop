import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { useLayoutEffect, useMemo, useState } from 'react';
import { transformMessage, type IMessageText, type TMessage } from '@/common/chat/chatLib';
import type { MessageTimeline } from '@/common/protocolBindings/MessageTimeline';
import type { MessageTimelineSegment } from '@/common/protocolBindings/MessageTimelineSegment';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { ipcBridge } from '@/common';
import { ConversationProvider } from '@/renderer/hooks/context/ConversationContext';
import { PreviewProvider } from '../../Preview';
import MessageList from '../MessageList';
import { MessageListProvider, useAddOrUpdateMessage, useMessageList } from '../hooks';
import { projectProcessSegments } from '../processSegments';
import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import { proseSourceIdentity } from '../proseSource';
import { useStreamingProseIngress } from '../streamingProse';
import ConversationProse, { ConversationProseContext } from './ConversationProse';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages, conversation } } } });
const session = parseConversationId('0190f5fe-7c00-7a00-8000-000000000091');
const otherSession = parseConversationId('0190f5fe-7c00-7a00-8000-000000000092');
const turn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000093');
const sourceId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000094');
const summaryId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000095');
const textMessage = (text: string, scope = session): IMessageText => ({
  id: 'canonical-reply', conversation_id: scope, turn_id: turn, message_id: sourceId, msg_id: sourceId,
  type: 'text', position: 'left', created_at: 2, content: { content: text },
});
const summary = (): TMessage => ({ id: 'terminal', type: 'agent_status', conversation_id: session,
  turn_id: turn, msg_id: summaryId, created_at: 3, content: { backend: 'nomi', status: 'prepared',
    turn_summary: true, turn_state: 'completed', started_at_ms: 1, finished_at_ms: 3,
    turn_outcome: { kind: 'answered', message_ids: [sourceId] } } });

const originalRaf = globalThis.requestAnimationFrame;
const originalCancel = globalThis.cancelAnimationFrame;
const originalSetTimeout = globalThis.setTimeout;
const originalClearTimeout = globalThis.clearTimeout;
const originalMedia = window.matchMedia;
const originalVisibility = Object.getOwnPropertyDescriptor(document, 'visibilityState');
const originalSecure = Object.getOwnPropertyDescriptor(window, 'isSecureContext');
let frames: Map<number, FrameRequestCallback>;
let now = 0;
let nextFrame = 0;
let nextTimer = 100_000;
let ingressTimers: Map<number, () => void>;
let restore: Array<() => void>;
let send: ReturnType<typeof useAddOrUpdateMessage>;
let canonical: TMessage[];
const canonicalText = () => canonical.find((message): message is IMessageText =>
  message.type === 'text' && message.position === 'left')?.content.content;

beforeEach(() => {
  now = 0;
  nextFrame = 0;
  nextTimer = 100_000;
  frames = new Map();
  ingressTimers = new Map();
  restore = [];
  const time = spyOn(performance, 'now').mockImplementation(() => now);
  restore.push(() => time.mockRestore());
  globalThis.requestAnimationFrame = callback => { frames.set(++nextFrame, callback); return nextFrame; };
  globalThis.cancelAnimationFrame = id => { frames.delete(id); };
  const scheduleTimer = ((...args: Parameters<typeof setTimeout>) => {
    const [callback, delay, ...values] = args;
    if (delay !== undefined && delay !== 0) return originalSetTimeout(...args);
    const id = ++nextTimer;
    ingressTimers.set(id, () => callback(...values));
    return id as unknown as ReturnType<typeof setTimeout>;
  }) as typeof setTimeout;
  const timers = spyOn(globalThis, 'setTimeout').mockImplementation(scheduleTimer);
  const cancelTimer = spyOn(globalThis, 'clearTimeout').mockImplementation(id => {
    if (!ingressTimers.delete(id as unknown as number)) originalClearTimeout(id);
  });
  restore.push(() => { timers.mockRestore(); cancelTimer.mockRestore(); });
  window.matchMedia = query => ({ media: query, matches: false,
    addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {},
  }) as unknown as MediaQueryList;
  Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
  const turnFiles = spyOn(ipcBridge.conversation.turnFiles, 'invoke').mockResolvedValue([]);
  const completed = spyOn(ipcBridge.conversation.turnCompleted, 'on').mockReturnValue(() => {});
  const reconnected = spyOn(ipcBridge.conversation.reconnected, 'on').mockReturnValue(() => {});
  const changed = spyOn(ipcBridge.agentPlatform.sessions.onAgentChanged, 'on').mockReturnValue(() => {});
  const fileContent = spyOn(ipcBridge.fileStream.contentUpdate, 'on').mockReturnValue(() => {});
  const tree = spyOn(ipcBridge.knowledge.onTreeChanged, 'on').mockReturnValue(() => {});
  const entry = spyOn(ipcBridge.knowledge.onEntryContentUpdated, 'on').mockReturnValue(() => {});
  const files = spyOn(ipcBridge.fs.listWorkspaceFiles, 'invoke').mockResolvedValue([]);
  const watchStart = spyOn(ipcBridge.workspaceOfficeWatch.start, 'invoke').mockResolvedValue(undefined);
  const watchStop = spyOn(ipcBridge.workspaceOfficeWatch.stop, 'invoke').mockResolvedValue(undefined);
  const watchFiles = spyOn(ipcBridge.workspaceOfficeWatch.fileAdded, 'on').mockReturnValue(() => {});
  restore.push(() => { turnFiles.mockRestore(); completed.mockRestore(); reconnected.mockRestore();
    changed.mockRestore(); fileContent.mockRestore(); tree.mockRestore(); entry.mockRestore(); files.mockRestore();
    watchStart.mockRestore(); watchStop.mockRestore(); watchFiles.mockRestore(); });
});

afterEach(() => {
  cleanup();
  restore.forEach(callback => callback());
  globalThis.requestAnimationFrame = originalRaf;
  globalThis.cancelAnimationFrame = originalCancel;
  window.matchMedia = originalMedia;
  if (originalVisibility) Object.defineProperty(document, 'visibilityState', originalVisibility);
  else Reflect.deleteProperty(document, 'visibilityState');
  if (originalSecure) Object.defineProperty(window, 'isSecureContext', originalSecure);
  else Reflect.deleteProperty(window, 'isSecureContext');
});

function frame(elapsed: number) {
  act(() => {
    now += elapsed;
    const pending = [...frames.values()];
    frames.clear();
    pending.forEach(callback => callback(now));
  });
}

function drainIngress() {
  act(() => {
    const pending = [...ingressTimers.values()];
    ingressTimers.clear();
    pending.forEach(callback => callback());
  });
}

const prose = (node: Element): string => [...node.querySelectorAll('.markdown-shadow')]
  .map(markdown => markdown.shadowRoot?.querySelector('.markdown-article')?.textContent ?? '').join('').trimEnd();

type Gate = { running: boolean; state: TurnDisclosureProcessState };
function ProseScene({ gate, publicView = false }: { gate: Gate; publicView?: boolean }) {
  const list = useMessageList();
  canonical = list;
  send = useAddOrUpdateMessage();
  const ingress = useStreamingProseIngress();
  const { sources, suppressed } = useMemo(() => {
    const sources = new Map<string, string>();
    const suppressed: string[] = [];
    for (const message of list) {
      if (message.type !== 'text') continue;
      const identity = proseSourceIdentity(message);
      if (!identity) continue;
      if (message.hidden || message.content.timeline?.segments.length === 0) suppressed.push(identity);
      else sources.set(identity, message.content.content);
    }
    return { sources, suppressed };
  }, [list]);
  const liveGeneration = ingress.generationFor(list);
  useLayoutEffect(() => {
    suppressed.forEach(identity => ingress.revoke(identity));
    ingress.commitSources(sources, liveGeneration);
  }, [ingress, sources, suppressed, liveGeneration]);
  const source = canonical.find((item): item is IMessageText => item.type === 'text' && item.position === 'left');
  if (!source || source.hidden) return <span data-testid='withdrawn' />;
  const projected = publicView ? source : projectProcessSegments([source], new Set()).at(-1);
  if (!projected || projected.type !== 'text') return null;
  return <ConversationProseContext.Provider value={{ sources, liveGeneration, turns: new Map([[turn, gate]]) }}>
    <div key={publicView ? 'public' : 'process'} data-testid='visible-prose'>
      <ConversationProse message={projected} text={projected.content.content.trim()} />
    </div>
  </ConversationProseContext.Provider>;
}

function Direct({ initial = [], scope = session, gate = { running: true, state: 'running' }, publicView = false }: {
  initial?: TMessage[]; scope?: typeof session; gate?: Gate; publicView?: boolean;
}) {
  return <I18nextProvider i18n={i18n}><MemoryRouter>
    <MessageListProvider key={scope} initialValue={initial} conversation={{ id: scope }}>
      <ProseScene gate={gate} publicView={publicView} />
    </MessageListProvider>
  </MemoryRouter></I18nextProvider>;
}

test('loaded history is immediately readable and real ingress animates only its new suffix', () => {
  const history = '已经显示的历史说明。';
  const suffix = '随后收到的实时内容。'.repeat(20);
  const page = render(<Direct initial={[textMessage(history)]} />);
  expect(prose(page.getByTestId('visible-prose'))).toBe(history);
  act(() => send(textMessage(suffix)));
  expect(canonicalText()).toBe(history);
  expect(frames.size).toBe(0);
  drainIngress();
  expect(canonicalText()).toBe(history + suffix);
  expect(frames.size).toBeGreaterThan(0);
  expect(prose(page.getByTestId('visible-prose'))).toBe(history);
  frame(100);
  const partial = prose(page.getByTestId('visible-prose'))!;
  expect(partial.startsWith(history)).toBe(true);
  expect(partial.length).toBeGreaterThan(history.length);
  expect(partial.length).toBeLessThan(history.length + suffix.length);
  frame(500);
  expect(prose(page.getByTestId('visible-prose'))).toBe(history + suffix);
});

test('collapsed live prose drains hidden appends and completed process history reopens fully without replay', async () => {
  const first = '折叠前已经收到的过程说明。'.repeat(20);
  const suffix = '折叠期间继续收到的正文😀。'.repeat(30);
  const finalId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000096');
  let finish = () => {};
  function Capture() {
    send = useAddOrUpdateMessage();
    canonical = useMessageList();
    return null;
  }
  function Timeline() {
    const [processing, setProcessing] = useState(true);
    finish = () => setProcessing(false);
    return <ConversationProvider value={{ conversation_id: session, type: 'nomi', isProcessing: processing,
      activeTurnId: turn, activeRequestMessageId: turn, isTurnStateHydrated: true }}>
      <MessageListProvider initialValue={[{ id: 'request', type: 'text', position: 'right', conversation_id: session,
        msg_id: turn, created_at: 1, content: { content: '请继续处理。' } }]}>
        <Capture /><MessageList />
      </MessageListProvider>
    </ConversationProvider>;
  }
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='collapsed-prose-flow' subscribeGlobalOpen={false}><Timeline /></PreviewProvider>
  </I18nextProvider></MemoryRouter>);
  await act(async () => {});
  act(() => send(textMessage(first)));
  drainIngress();
  frame(32);
  expect(prose(page.getByTestId('process-narration')).length).toBeLessThan(first.length);
  const disclosure = () => page.container.querySelector('.turn-process-disclosure__toggle')!;
  fireEvent.click(disclosure());
  expect(page.queryByTestId('process-narration')).toBeNull();
  act(() => send(textMessage(suffix)));
  drainIngress();
  frame(600);
  expect(page.queryByTestId('process-narration')).toBeNull();
  expect(canonicalText()).toBe(first + suffix);
  act(() => {
    send({ ...textMessage('任务已完成。'), id: 'final-answer', message_id: finalId, msg_id: finalId });
    const terminal = summary();
    if (terminal.type !== 'agent_status') throw new Error('expected canonical terminal fixture');
    send({ ...terminal, content: { ...terminal.content, turn_outcome: { kind: 'answered', message_ids: [finalId] } } });
    finish();
  });
  drainIngress();
  await act(async () => {});
  frame(600);
  expect(disclosure().getAttribute('aria-expanded')).toBe('false');
  fireEvent.click(disclosure());
  expect(prose(page.getByTestId('process-narration'))).toBe(first + suffix);
  frame(16);
  expect(prose(page.getByTestId('process-narration'))).toBe(first + suffix);
});

test('canonical narration ranges and a public remount keep the same already-visible source prefix', () => {
  const prefix = '前段已经显示😀。';
  const suffix = '恢复后的 canonical 正文😀。'.repeat(40);
  const text = prefix + suffix;
  const closed = { segment_id: 'narration-first', group_id: 'process-root', kind: 'narration' as const,
    state: 'completed' as const, first_seq: 2, last_seq: 3, start_utf16: 0, end_utf16: prefix.length };
  const history = textMessage(prefix);
  history.content.timeline = { through_seq: 3, segments: [closed] };
  const message = textMessage(suffix);
  message.content.timeline = { through_seq: 5, segments: [closed, { segment_id: 'narration-resumed', group_id: 'process-first',
    kind: 'narration', state: 'running', first_seq: 5, last_seq: 5, start_utf16: prefix.length, end_utf16: prefix.length + 2 }] };
  const page = render(<Direct initial={[history]} />);
  expect(prose(page.getByTestId('visible-prose'))).toBe(prefix);
  act(() => send(message));
  drainIngress();
  expect(prose(page.getByTestId('visible-prose'))).toBe('');
  frame(120);
  const before = prose(page.getByTestId('visible-prose'))!;
  expect(before.length).toBeGreaterThan(0);
  expect(before.length).toBeLessThan(suffix.length);
  page.rerender(<Direct publicView gate={{ running: false, state: 'completed' }} />);
  expect(prose(page.getByTestId('visible-prose'))).toBe(prefix + before);
  expect(canonicalText()).toBe(text);
  frame(480);
  expect(prose(page.getByTestId('visible-prose'))).toBe(text);
});

test('authoritative replacement and withdrawal cancel the old reveal instead of replaying superseded text', () => {
  const page = render(<Direct />);
  act(() => send(textMessage('将被替换的旧输出。'.repeat(30))));
  drainIngress();
  frame(80);
  const replacement = { ...textMessage('已经校正的正文。'), content: { content: '已经校正的正文。', replace: true } };
  act(() => send(replacement));
  expect(prose(page.getByTestId('visible-prose'))).toBe('已经校正的正文。');
  expect(frames.size).toBe(0);
  act(() => send({ ...replacement, hidden: true, content: { content: '', replace: true } }));
  expect(page.queryByTestId('visible-prose')).toBeNull();
  frame(1000);
  expect(page.container.textContent).not.toContain('旧输出');
  expect(canonical[0].hidden).toBe(true);
  expect(frames.size).toBe(0);
  act(() => send(textMessage('迟到的旧输出。')));
  drainIngress();
  expect(page.queryByTestId('visible-prose')).toBeNull();
  expect(canonical[0].hidden).toBe(true);
  expect(frames.size).toBe(0);
});

test('an authoritative canceled Turn flushes received text and late ingress cannot restart its reveal', () => {
  const text = '停止前已经收到的内容。'.repeat(30);
  const page = render(<Direct />);
  act(() => send(textMessage(text)));
  drainIngress();
  frame(64);
  expect(prose(page.getByTestId('visible-prose'))!.length).toBeLessThan(text.length);
  page.rerender(<Direct gate={{ running: false, state: 'canceled' }} />);
  expect(prose(page.getByTestId('visible-prose'))).toBe(text);
  expect(frames.size).toBe(0);
  act(() => send(textMessage('迟到的已接收片段。')));
  drainIngress();
  expect(prose(page.getByTestId('visible-prose'))).toBe(text + '迟到的已接收片段。');
  expect(frames.size).toBe(0);
});

test('changing Session disposes old reveal frames and new history never inherits live provenance', () => {
  const page = render(<Direct />);
  act(() => send(textMessage('旧会话流式正文。'.repeat(20))));
  drainIngress();
  frame(32);
  const history = '新会话的完整历史正文。';
  page.rerender(<Direct scope={otherSession} initial={[textMessage(history, otherSession)]} />);
  expect(prose(page.getByTestId('visible-prose'))).toBe(history);
  expect(frames.size).toBe(0);
  frame(1000);
  expect(prose(page.getByTestId('visible-prose'))).toBe(history);
});

test.each(['same ingress batch', 'after visible process narration'])('actual MessageList delivers an atomic final report %s without delaying settlement or canonical copy', async timing => {
  const report = '已经验收的完整交付结果😀。'.repeat(40);
  const write = spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
  restore.push(() => write.mockRestore());
  Object.defineProperty(window, 'isSecureContext', { configurable: true, value: true });
  let finish = () => {};
  function ActualTimeline() {
    const [processing, setProcessing] = useState(true);
    return <ConversationProvider value={{ conversation_id: session, type: 'nomi', isProcessing: processing,
      activeTurnId: turn, activeRequestMessageId: turn, isTurnStateHydrated: true }}>
      <MessageListProvider initialValue={[{ id: 'request', type: 'text', position: 'right', conversation_id: session,
        msg_id: turn, created_at: 1, content: { content: '请完成任务。' } }]}>
        <Capture finishTurn={() => setProcessing(false)} />
        <MessageList />
        <output data-testid='busy'>{String(processing)}</output>
      </MessageListProvider>
    </ConversationProvider>;
  }
  function Capture({ finishTurn }: { finishTurn: () => void }) {
    send = useAddOrUpdateMessage();
    canonical = useMessageList();
    finish = finishTurn;
    return null;
  }
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='streaming-prose-flow' subscribeGlobalOpen={false}><ActualTimeline /></PreviewProvider>
  </I18nextProvider></MemoryRouter>);
  let shownBeforeTerminal = '';
  if (timing === 'after visible process narration') {
    act(() => send(textMessage(report)));
    drainIngress();
    frame(120);
    shownBeforeTerminal = prose(page.getByTestId('process-narration'))!;
    expect(shownBeforeTerminal.length).toBeGreaterThan(0);
    expect(shownBeforeTerminal.length).toBeLessThan(report.length);
  }
  act(() => {
    if (timing === 'same ingress batch') send(textMessage(report));
    send(summary());
    finish();
  });
  drainIngress();
  const answer = () => page.container.querySelector('[data-testid="message-text-left"] [data-testid="message-text-content"]')!;
  expect(answer()).not.toBeNull();
  expect(prose(answer())).toBe(shownBeforeTerminal);
  expect(page.getByTestId('busy').textContent).toBe('false');
  expect(page.container.querySelector('.turn-process-disclosure--live')).toBeNull();
  expect(canonicalText()).toBe(report);
  await act(async () => fireEvent.click(page.getByTestId('message-text-left').querySelector('[data-testid="message-copy-action"]')!));
  expect(write).toHaveBeenCalledWith(report);
  expect(prose(answer())!.length).toBeLessThan(report.length);
  frame(600);
  expect(prose(answer())).toBe(report);
  expect(page.getByTestId('busy').textContent).toBe('false');
});

test.each(['真实思考。', '', '[Private reasoning omitted from replay]'])(
  'canonical narration stays complete across reasoning %j in live display and reopened history, retaining tool and message boundaries', async reasoning => {
    const thinkingId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000111');
    const independentId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000112');
    const finalId = parseMessageId('0190f5fe-7c00-7a00-8000-000000000113');
    const first = '我将';
    const middle = '按照文档安装 **CL';
    const tail = 'I**。请看[官方文档](https://example.com/docs)。\n\n```sh\nbun --version\n```\n\n| 项目 | 状态 |\n| --- | --- |\n| CLI | 已安装 |';
    const complete = first + middle + tail;
    const nextReasoning = reasoning === '真实思考。' ? '继续思考。' : '';
    const afterTool = '工具完成后继续。';
    const independent = '独立消息保留自己的正文边界。';
    const timeline = (through: number, segments: MessageTimelineSegment[]): MessageTimeline => ({ through_seq: through, segments });
    const narration = (end: number, seq: number, state: 'running' | 'completed' = 'running'): MessageTimelineSegment => ({
      segment_id: 'continuous-narration', group_id: `process:${turn}`, kind: 'narration', state,
      first_seq: 2, last_seq: seq, start_utf16: 0, end_utf16: end,
    });
    const request: TMessage = { id: 'request', type: 'text', position: 'right', conversation_id: session,
      msg_id: turn, created_at: 1, content: { content: '请安装并验证。' } };
    let finish = () => {};
    function Capture({ finishTurn }: { finishTurn: () => void }) {
      send = useAddOrUpdateMessage();
      canonical = useMessageList();
      finish = finishTurn;
      return null;
    }
    function Timeline({ history }: { history?: TMessage[] }) {
      const [processing, setProcessing] = useState(!history);
      return <ConversationProvider value={{ conversation_id: session, type: 'nomi', isProcessing: processing,
        activeTurnId: processing ? turn : undefined, activeRequestMessageId: processing ? turn : undefined, isTurnStateHydrated: true }}>
        <MessageListProvider initialValue={history ?? [request]}>
          <Capture finishTurn={() => setProcessing(false)} /><MessageList />
        </MessageListProvider>
      </ConversationProvider>;
    }
    const view = (history?: TMessage[]) => <MemoryRouter><I18nextProvider i18n={i18n}>
      <PreviewProvider persistNamespace='canonical-complete-narration' subscribeGlobalOpen={false}>
        <Timeline key={history ? 'reopened' : 'live'} history={history} />
      </PreviewProvider>
    </I18nextProvider></MemoryRouter>;
    const page = render(view());
    await act(async () => {});
    const appendNarration = (content: string, seq: number, end: number) => ({ ...textMessage(content), content: { content,
      timeline: timeline(seq, [narration(end, seq)]) } });
    act(() => send(appendNarration(first, 2, first.length)));
    drainIngress();
    const narrationNode = page.getByTestId('process-narration');
    act(() => send({ id: 'interleaved-thinking', type: 'thinking', position: 'left', conversation_id: session,
      turn_id: turn, msg_id: thinkingId, created_at: 3, content: { content: reasoning, status: 'done', timeline: timeline(3, [{
        segment_id: 'thinking-between-words', group_id: 'process:continuous-narration', kind: 'reasoning', state: 'completed',
        first_seq: 3, last_seq: 3, start_utf16: 0, end_utf16: reasoning.length,
      }]) } }));
    drainIngress();
    if (!reasoning || reasoning.startsWith('[Private')) expect(page.container.querySelector('.turn-process-group')).toBeNull();
    act(() => send(appendNarration(middle, 4, first.length + middle.length)));
    drainIngress();
    act(() => send({ id: 'interleaved-thinking', type: 'thinking', position: 'left', conversation_id: session,
      turn_id: turn, msg_id: thinkingId, content: { content: nextReasoning, status: 'done',
        timeline: timeline(5, [{ segment_id: 'thinking-between-words',
        group_id: 'process:continuous-narration', kind: 'reasoning', state: 'completed', first_seq: 3,
        last_seq: 5, start_utf16: 0, end_utf16: reasoning.length + nextReasoning.length }]),
      } }));
    act(() => send(appendNarration(tail, 6, complete.length)));
    drainIngress();
    frame(1000);
    expect(page.getAllByTestId('process-narration')).toHaveLength(1);
    expect(page.getByTestId('process-narration')).toBe(narrationNode);
    const assertCompleteMarkdown = () => {
      const nodes = page.getAllByTestId('process-narration');
      const root = nodes[0].querySelector('.markdown-shadow')!.shadowRoot!;
      expect(root.querySelector('strong')?.textContent).toBe('CLI');
      expect(root.querySelector('a')?.getAttribute('href')).toBe('https://example.com/docs');
      expect(root.querySelector('pre')?.textContent).toContain('bun --version');
      expect(root.querySelector('table')?.textContent).toContain('已安装');
      expect(prose(nodes[0])).toStartWith('我将按照文档安装 CLI。');
      return nodes;
    };
    assertCompleteMarkdown();
    act(() => {
      send({ type: 'canonical_timeline', conversation_id: session, turn_id: turn, updates: [{
        source_message_id: sourceId, timeline: timeline(7, [narration(complete.length, 7, 'completed')]),
      }] });
      send({ id: 'true-tool-boundary', type: 'tool_call', conversation_id: session, turn_id: turn,
        created_at: 7, content: { call_id: 'installation', name: 'exec_command', status: 'completed', description: 'bun --version',
          timeline: timeline(8, [{ segment_id: 'installation-tool', group_id: 'process:continuous-narration', first_seq: 7,
            last_seq: 8, kind: 'tool', state: 'completed', call_id: 'installation' }]) } });
      send({ ...textMessage(afterTool), content: { content: afterTool, timeline: timeline(10, [narration(complete.length, 7, 'completed'), {
        segment_id: 'after-tool-narration', group_id: 'process:continuous-narration', first_seq: 9, last_seq: 10,
        kind: 'narration', state: 'completed', start_utf16: complete.length, end_utf16: complete.length + afterTool.length,
      }]) } });
      send({ ...textMessage(independent), id: 'independent-message', msg_id: independentId, message_id: independentId,
        created_at: 11, content: { content: independent, timeline: timeline(12, [{
          segment_id: 'independent-narration', group_id: 'process:after-tool-narration', first_seq: 11, last_seq: 12,
          kind: 'narration', state: 'completed', start_utf16: 0, end_utf16: independent.length,
        }]) } });
    });
    drainIngress();
    frame(1000);
    const liveNodes = assertCompleteMarkdown();
    const liveText = liveNodes.map(prose);
    expect(liveText).toEqual([liveText[0], afterTool, independent]);
    const group = page.container.querySelector('[data-process-group-id="process:continuous-narration"]')!;
    expect(liveNodes[0].compareDocumentPosition(group) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(group.compareDocumentPosition(liveNodes[1]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(group.querySelector('button')!);
    expect(page.container.querySelector('[data-tool-call-id="installation"]')).not.toBeNull();
    const assertReasoning = (node: Element) => {
      const toggle = node.querySelector('[data-thinking-process-header]');
      if (!nextReasoning) { expect(toggle).toBeNull(); return; }
      expect(toggle).not.toBeNull();
      fireEvent.click(toggle!);
      expect(node.querySelector('.markdown-shadow')?.shadowRoot?.textContent).toContain(reasoning + nextReasoning);
    };
    assertReasoning(group);
    act(() => {
      send({ ...textMessage('安装完成。'), id: 'final-answer', message_id: finalId, msg_id: finalId, created_at: 13 });
      const terminal = summary();
      if (terminal.type !== 'agent_status') throw new Error('expected canonical terminal fixture');
      send({ ...terminal, created_at: 14, content: { ...terminal.content, turn_outcome: { kind: 'answered', message_ids: [finalId] } } });
      finish();
    });
    drainIngress();
    await act(async () => {});
    const history = canonical;
    page.rerender(view(history));
    expect(page.queryByTestId('process-narration')).toBeNull();
    fireEvent.click(page.container.querySelector('.turn-process-disclosure__toggle')!);
    const reopenedNodes = assertCompleteMarkdown();
    expect(reopenedNodes.map(prose)).toEqual(liveText);
    expect(reopenedNodes).toHaveLength(3);
    const reopenedGroup = page.container.querySelector('[data-process-group-id="process:continuous-narration"]')!;
    expect(reopenedNodes[0].compareDocumentPosition(reopenedGroup) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(reopenedGroup.compareDocumentPosition(reopenedNodes[1]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(reopenedGroup.querySelector('button')!);
    expect(page.container.querySelector('[data-tool-call-id="installation"]')).not.toBeNull();
    assertReasoning(reopenedGroup);
    expect(page.container.querySelector('[data-testid="message-text-left"]')?.closest('.turn-process-disclosure')).toBeNull();
    await act(async () => {});
  }
);

test('a text wire frame carries its new canonical boundary atomically without a later timeline notification', async () => {
  const first = '第一条消息保持 **CLI**。';
  const second = '第二条消息也有[链接](https://example.com/second)。';
  const firstSegment: MessageTimelineSegment = { segment_id: 'first-source', group_id: `process:${turn}`,
    first_seq: 2, last_seq: 2, kind: 'narration', state: 'running', start_utf16: 0, end_utf16: first.length };
  const initial: TMessage[] = [
    { id: 'request', type: 'text', position: 'right', conversation_id: session,
      msg_id: turn, created_at: 1, content: { content: '继续处理。' } },
    { ...textMessage(first), content: { content: first, timeline: { through_seq: 2, segments: [firstSegment] } } },
  ];
  function Capture() {
    send = useAddOrUpdateMessage();
    canonical = useMessageList();
    return null;
  }
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='atomic-text-timeline' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: session, type: 'nomi', isProcessing: true,
        activeTurnId: turn, activeRequestMessageId: turn, isTurnStateHydrated: true }}>
        <MessageListProvider initialValue={initial}><Capture /><MessageList /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>);
  await act(async () => {});
  const firstNode = page.getByTestId('process-narration');
  const timeline: MessageTimeline = { through_seq: 4, segments: [
    { ...firstSegment, state: 'completed', last_seq: 4 },
    { segment_id: 'second-source', group_id: 'process:first-source', first_seq: 4, last_seq: 4,
      kind: 'narration', state: 'running', start_utf16: first.length, end_utf16: first.length + second.length },
  ] };
  const wire = transformMessage({ type: 'content', conversation_id: session, msg_id: sourceId, turn_id: turn,
    data: { content: second, step: 1, timeline }, created_at: 4 });
  act(() => send(wire));
  drainIngress();
  // This checks the first render after the only ingress frame, before any
  // animation frame or optional standalone canonical_timeline notification.
  expect(page.getAllByTestId('process-narration')).toHaveLength(2);
  expect(page.getAllByTestId('process-narration')[0]).toBe(firstNode);
  expect(prose(firstNode)).toBe('第一条消息保持 CLI。');
  frame(1000);
  const nodes = page.getAllByTestId('process-narration');
  expect(prose(nodes[0])).toBe('第一条消息保持 CLI。');
  expect(prose(nodes[1])).toBe('第二条消息也有链接。');
  expect(nodes[1].querySelector('.markdown-shadow')?.shadowRoot?.querySelector('a')?.getAttribute('href'))
    .toBe('https://example.com/second');
  expect(canonicalText()).toBe(first + second);
});

test.each([
  { label: 'same text block around reasoning', parts: ['我将安装 **CLI**。'], count: 1 },
  { label: 'separate provider blocks', parts: ['第一条 **CLI**。', '[第二条消息](https://example.com/second)'], count: 2 },
  { label: 'complete source with trimmed outer whitespace', parts: [' \n第一条 **CLI**。', '[第二条消息](https://example.com/second)\n '], count: 2 },
  { label: 'filtered source', parts: ['<think>PRIVATE</think>', '公共 **CLI**。'], count: 1 },
  { label: 'separate blocks that form JSON', parts: ['{"first":', '"value"}'], count: 2 },
])('an answered public Message preserves canonical Markdown boundaries for $label without changing its identity or copy', async ({ label, parts, count }) => {
  const body = parts.join('');
  const jsonBlocks = label === 'separate blocks that form JSON';
  let offset = 0;
  const segments: MessageTimelineSegment[] = parts.map((part, index) => {
    const start = offset;
    offset += part.length;
    return { segment_id: `public-source-${index}`, group_id: index ? `process:public-source-${index - 1}` : `process:${turn}`,
      first_seq: 2 + index * 4, last_seq: 5 + index * 4, kind: 'narration', state: 'completed', start_utf16: start, end_utf16: offset };
  });
  const source = { ...textMessage(body), content: { content: body, timeline: { through_seq: 12, segments } } };
  const initial: TMessage[] = [
    { id: 'request', type: 'text', position: 'right', conversation_id: session,
      msg_id: turn, created_at: 1, content: { content: '请给出答案。' } },
    source,
    { id: 'public-interleaved-thinking', type: 'thinking', conversation_id: session, turn_id: turn,
      msg_id: parseMessageId('0190f5fe-7c00-7a00-8000-000000000115'), created_at: 3,
      content: { content: '正文期间的思考。', status: 'done', timeline: { through_seq: 12, segments: [{
        segment_id: 'public-thinking', group_id: 'process:public-source-0', first_seq: 3, last_seq: 4,
        kind: 'reasoning', state: 'completed', start_utf16: 0, end_utf16: '正文期间的思考。'.length,
      }] } } },
    summary(),
  ];
  const write = spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
  restore.push(() => write.mockRestore());
  Object.defineProperty(window, 'isSecureContext', { configurable: true, value: true });
  function Capture() { canonical = useMessageList(); return null; }
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <PreviewProvider persistNamespace='public-canonical-source-boundaries' subscribeGlobalOpen={false}>
      <ConversationProvider value={{ conversation_id: session, type: 'nomi', isProcessing: false, isTurnStateHydrated: true }}>
        <MessageListProvider initialValue={initial}><Capture /><MessageList /></MessageListProvider>
      </ConversationProvider>
    </PreviewProvider>
  </I18nextProvider></MemoryRouter>);
  await act(async () => {});
  const answer = page.getByTestId('message-text-left');
  expect(page.getAllByTestId('message-text-left')).toHaveLength(1);
  expect(answer.getAttribute('data-message-business-id')).toBe(sourceId);
  expect(answer.closest('.turn-process-disclosure')).toBeNull();
  const roots = [...answer.querySelectorAll('.markdown-shadow')].map(node => node.shadowRoot!);
  expect(roots).toHaveLength(count);
  expect(roots.flatMap(root => [...root.querySelectorAll('strong')].map(node => node.textContent))).toEqual(jsonBlocks ? [] : ['CLI']);
  expect(roots.map(root => root.textContent).join('')).not.toContain('PRIVATE');
  if (count === 2 && !jsonBlocks) expect(roots[1].querySelector('a')?.getAttribute('href')).toBe('https://example.com/second');
  expect(canonicalText()).toBe(body);
  await act(async () => fireEvent.click(answer.querySelector('[data-testid="message-copy-action"]')!));
  expect(write).toHaveBeenCalledWith(jsonBlocks ? JSON.stringify({ first: 'value' }, null, 2)
    : parts[0].startsWith('<think>') ? parts[1] : body.trim());
});
