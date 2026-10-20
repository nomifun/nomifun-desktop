import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { database } from '@/common/adapter/ipcBridge';
import type { IMessageToolCall } from '@/common/chat/chatLib';
import type { ToolDetailPage, ToolDetailRequest } from '@/common/chat/toolDetails';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import ProcessTraceItem from './ProcessTraceItem';
import { ToolDetailsProvider } from './ToolDetails';
import { emitter } from '@/renderer/utils/emitter';
import { ipcBridge } from '@/common';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { messages } } } });
const session = parseConversationId('0190f5fe-7c00-7a00-8000-000000000021');
const turn = parseMessageId('0190f5fe-7c00-7a00-8000-000000000022');
const tool = (overrides: Partial<IMessageToolCall['content']> = {}): IMessageToolCall => ({ id: 'command', type: 'tool_call', conversation_id: session,
  turn_id: turn, position: 'left', content: { name: 'start_process', call_id: 'call', status: 'completed', description: 'Run validation', result_seq: 8, ...overrides } });
const page = (request: ToolDetailRequest, overrides: Partial<ToolDetailPage> = {}): ToolDetailPage => ({ turn_id: turn, call_id: request.call_id,
  call_event_id: 'canonical-call', through_seq: request.through_seq ?? 10, result_seq: 8, section: request.section,
  availability: 'available', content: request.section === 'arguments' ? 'bun test\nprintf "done"' : '2 tests passed',
  format: 'text', offset: 0, total_bytes: 22, eof: true, non_text_parts: 0, ...overrides });
const element = (item: IMessageToolCall) => <I18nextProvider i18n={i18n}><ToolDetailsProvider scope={session}><ProcessTraceItem item={item} /></ToolDetailsProvider></I18nextProvider>;
const show = (item: IMessageToolCall) => render(element(item));
let restore = () => {};
let reconnected: () => void;
let restoreReconnect = () => {};
beforeEach(() => {
  const observer = spyOn(ipcBridge.conversation.reconnected, 'on').mockImplementation(listener => {
    reconnected = () => listener(); return () => {};
  });
  restoreReconnect = () => observer.mockRestore();
});
afterEach(() => { cleanup(); restore(); restoreReconnect(); });

test('command details are lazy, readable, independently paginated and never written into the message', async () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockImplementation(async request => page(request,
    request.section === 'arguments' ? { working_directory: '/workspace' }
      : request.cursor ? { content: '\nlast line', offset: 14, next_cursor: undefined }
        : { next_cursor: 'opaque-next', eof: false, retention: { retained_bytes: 14, dropped_bytes: 2, decode_errors: 1, next_cursor: 16, source_encoding: 'utf-8' } }));
  restore = () => read.mockRestore();
  const item = tool(); const saved = JSON.stringify(item);
  const view = show(item);
  expect(read).not.toHaveBeenCalled();
  expect(view.container.querySelector('pre')).toBeNull();
  fireEvent.click(view.getByRole('button'));
  await waitFor(() => expect(view.container.textContent).toContain('bun test\nprintf "done"'));
  expect(view.container.textContent).toContain('Working directory: /workspace');
  expect(view.container.textContent).toContain('original observation dropped 2 bytes');
  expect(view.container.textContent).toContain('1 decoding errors');
  expect(view.queryByRole('tab')).toBeNull();
  fireEvent.click(view.getByRole('button', { name: 'Load more' }));
  await waitFor(() => expect(view.container.textContent).toContain('last line'));
  expect(read.mock.calls.at(-1)?.[0]).toMatchObject({ cursor: 'opaque-next', through_seq: 10, expected_result_seq: 8 });
  expect(JSON.stringify(item)).toBe(saved);
});

test('read/write files default to summaries while retaining deliberate access to their payloads', async () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockImplementation(async request => page(request, { content: 'file body' }));
  restore = () => read.mockRestore();
  const view = show(tool({ name: 'read_file', description: '/workspace/src/app.ts' }));
  expect(view.container.textContent).toContain('app.ts');
  expect(view.container.textContent).not.toContain('file body');
  expect(read).not.toHaveBeenCalled();
  fireEvent.click(view.getByRole('button'));
  await waitFor(() => expect(view.container.textContent).toContain('file body'));
});

test('a late trusted result supersedes an interrupted observation without inferring success from output JSON', async () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockImplementation(async request => page(request, { content: '{"success":false}', format: 'json' }));
  restore = () => read.mockRestore();
  const view = show(tool({ name: 'mcp__server__inspect', timeline: { through_seq: 7, segments: [{ segment_id: 'call', group_id: 'phase', first_seq: 1, last_seq: 7,
    kind: 'tool', call_id: 'call', state: 'interrupted' }] } }));
  expect(view.container.textContent).not.toContain('Result unconfirmed');
  fireEvent.click(view.getByRole('button'));
  await waitFor(() => expect(view.container.textContent).toContain('"success": false'));
  expect(read.mock.calls[0][0].through_seq).toBe(8);
  expect(view.container.querySelector('.turn-process-trace__row--failed')).toBeNull();
});

test.each(['pending', 'unconfirmed', 'available'] as const)('%s output is not confused with another outcome', async availability => {
  const read = spyOn(database.getToolDetails, 'invoke').mockImplementation(async request => page(request, { availability, content: '', total_bytes: 0 }));
  restore = () => read.mockRestore();
  const view = show(tool()); fireEvent.click(view.getByRole('button'));
  await waitFor(() => expect(view.container.textContent).toContain(availability === 'available' ? messages.toolDetails.empty
    : availability === 'pending' ? messages.toolDetails.pending : messages.toolResultUnconfirmed));
});

test('closing suspends late observers and Session deletion invalidates retained payload pages', async () => {
  const pending: Array<() => void> = [];
  const read = spyOn(database.getToolDetails, 'invoke').mockImplementation(request => new Promise(resolve => {
    pending.push(() => resolve(page(request)));
  }));
  restore = () => read.mockRestore();
  const view = show(tool()); const toggle = view.getByRole('button');
  fireEvent.click(toggle);
  expect(read).toHaveBeenCalledTimes(2);
  fireEvent.click(toggle);
  await act(async () => { pending.forEach(resolve => resolve()); });
  expect(view.container.querySelector('pre')).toBeNull();
  fireEvent.click(toggle);
  await waitFor(() => expect(view.container.textContent).toContain('2 tests passed'));
  expect(read).toHaveBeenCalledTimes(2);
  fireEvent.click(toggle);
  act(() => emitter.emit('conversation.deleted', session));
  read.mockImplementation(async request => page(request));
  fireEvent.click(toggle);
  await waitFor(() => expect(read).toHaveBeenCalledTimes(4));
});

test('a reconnect clears visible payloads and old pending pages while preserving the row choice', async () => {
  const pending: Array<() => void> = [];
  const read = spyOn(database.getToolDetails, 'invoke').mockImplementation(request => new Promise(resolve => {
    pending.push(() => resolve(page(request, { content: 'old backend payload' })));
  }));
  restore = () => read.mockRestore();
  const view = show(tool()); const toggle = view.getByRole('button');
  fireEvent.click(toggle);
  read.mockImplementation(async request => page(request, { content: 'current backend payload' }));
  act(() => reconnected());
  await waitFor(() => expect(view.container.textContent).toContain('current backend payload'));
  await act(async () => { pending.forEach(resolve => resolve()); });
  expect(toggle.getAttribute('aria-expanded')).toBe('true');
  expect(view.container.textContent).not.toContain('old backend payload');
  expect(read).toHaveBeenCalledTimes(4);
});

test('unrelated Session head growth preserves an opened result snapshot, loaded pages and reading position', async () => {
  const timeline = (through_seq: number) => ({ through_seq, segments: [{ segment_id: 'call', group_id: 'phase', first_seq: 1, last_seq: 8,
    kind: 'tool' as const, call_id: 'call', state: 'completed' as const }] });
  const read = spyOn(database.getToolDetails, 'invoke').mockImplementation(async request => {
    if (request.expected_result_seq === 9) return page(request, { result_seq: 9, content: request.section === 'output' ? 'new result' : 'new command' });
    if (request.section === 'arguments') return page(request);
    return page(request, request.cursor === 'third' ? { content: 'third', offset: 13 }
      : request.cursor === 'second' ? { content: 'second\n', offset: 6, next_cursor: 'third', eof: false }
        : { content: 'first\n', offset: 0, next_cursor: 'second', eof: false });
  });
  restore = () => read.mockRestore();
  const initial = tool({ timeline: timeline(10) });
  const view = show(initial);
  fireEvent.click(view.getByRole('button'));
  await waitFor(() => expect(view.container.textContent).toContain('first\n'));
  fireEvent.click(view.getByRole('button', { name: 'Load more' }));
  await waitFor(() => expect(view.container.textContent).toContain('first\nsecond\n'));
  const detail = view.container.querySelector<HTMLElement>('.turn-process-trace-detail')!;
  const output = view.container.querySelector('[data-tool-detail-section="output"] pre');
  detail.scrollTop = 27;
  await act(async () => view.rerender(element(tool({ timeline: timeline(50) }))));
  expect(read).toHaveBeenCalledTimes(3);
  expect(view.container.querySelector('.turn-process-trace-detail') === detail).toBe(true);
  expect(view.container.querySelector('[data-tool-detail-section="output"] pre') === output).toBe(true);
  expect(detail.scrollTop).toBe(27);
  expect(output?.textContent).toBe('first\nsecond\n');
  fireEvent.click(view.getByRole('button', { name: 'Load more' }));
  await waitFor(() => expect(view.container.textContent).toContain('first\nsecond\nthird'));
  expect(read.mock.calls.at(-1)?.[0]).toMatchObject({ through_seq: 10, expected_result_seq: 8, cursor: 'third' });

  await act(async () => view.rerender(element(tool({ timeline: timeline(60), result_seq: 9 }))));
  await waitFor(() => expect(view.container.textContent).toContain('new result'));
  expect(read).toHaveBeenCalledTimes(6);
  expect(view.container.querySelector('[data-tool-detail-section="output"] pre')?.textContent).toBe('new result');
  expect(read.mock.calls.slice(-2).every(([request]) => request.through_seq === 60 && request.expected_result_seq === 9)).toBe(true);
  expect(initial.content.timeline?.through_seq).toBe(10);
});

test('a pending call refreshes when its committed result identity arrives', async () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockImplementation(async request => request.expected_result_seq === undefined
    ? page(request, { result_seq: undefined, availability: 'pending', content: '', total_bytes: 0 }) : page(request));
  restore = () => read.mockRestore();
  const view = show(tool({ status: 'running', result_seq: undefined }));
  fireEvent.click(view.getByRole('button'));
  await waitFor(() => expect(view.container.textContent).toContain(messages.toolDetails.pending));
  await act(async () => view.rerender(element(tool())));
  await waitFor(() => expect(view.container.textContent).toContain('2 tests passed'));
  expect(view.container.textContent).not.toContain(messages.toolDetails.pending);
  expect(read).toHaveBeenCalledTimes(4);
});
