import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { useLayoutEffect, useMemo, useState } from 'react';
import type { IMessageText, TMessage } from '@/common/chat/chatLib';
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

const prose = (node: Element): string => node.querySelector('.markdown-shadow')?.shadowRoot?.querySelector('.markdown-article')?.textContent?.trimEnd() ?? '';

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
