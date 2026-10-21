import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { database } from '@/common/adapter/ipcBridge';
import type { IMessageToolCall } from '@/common/chat/chatLib';
import type { ToolDetailPage, ToolDetailRequest } from '@/common/chat/toolDetails';
import type { ToolFileSummary } from '@/common/chat/fileSummary';
import { parseConversationId, parseMessageId } from '@/common/types/ids';
import { setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
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
type ProcessItem = Parameters<typeof ProcessTraceItem>[0]['item'];
const element = (item: ProcessItem) => <I18nextProvider i18n={i18n}><ToolDetailsProvider scope={session}><ProcessTraceItem item={item} workspaceRoots={['/workspace']} /></ToolDetailsProvider></I18nextProvider>;
const show = (item: ProcessItem) => render(element(item));
const file = (call_id: string, operation: ToolFileSummary['operation'], files: ToolFileSummary['files'], overrides: Partial<IMessageToolCall['content']> = {}) => tool({
  call_id, name: operation === 'patch' ? 'apply_patch' : operation === 'read' ? 'read_file' : operation === 'write' ? 'write_file' : `${operation}_path`,
  capability_id: 'workspace.files', action_id: `workspace.files/${operation}`, description: files[0]?.path,
  file_summary: { operation, files }, ...overrides,
});
const fileRun = (messages: IMessageToolCall[]): ProcessItem => ({
  type: 'tool_summary', id: 'file-run', messages, sourceMessageIds: [], created_at: 1,
});
let restore = () => {};
let reconnected: () => void;
let restoreReconnect = () => {};
beforeEach(() => {
  // Deletion is a renderer-wide notification. When the full conversation UI
  // is loaded, its draft cleanup listener requires the same dataset bootstrap
  // that production performs before any Session deletion can be observed.
  setBrowserStorageGeneration('0190f5fe-7c00-7a00-8000-000000000020');
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

test('confirmed file reads expand only a path list and never request arguments or file bodies', () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockRejectedValue(new Error('File payload access must remain unused'));
  restore = () => read.mockRestore();
  const item = file('read-call', 'read', [{ path: '/workspace/src/app.ts' }]);
  Object.assign(item.content, { arguments: { path: '/workspace/src/app.ts', content: 'PRIVATE FILE BODY' }, output: 'PRIVATE FILE BODY' });
  const before = JSON.stringify(item);
  const view = show(item);
  const toggle = view.getByRole('button', { name: 'Read 1 file' });
  expect(toggle.getAttribute('aria-expanded')).toBe('false');
  expect(Boolean(view.container.querySelector('.turn-file-activity__list'))).toBe(false);
  expect(read).not.toHaveBeenCalled();
  fireEvent.click(toggle);
  expect(view.getByRole('listitem').textContent).toContain('src/app.ts');
  expect(view.container.querySelector('.turn-file-activity__path')?.getAttribute('title')).toBe('/workspace/src/app.ts');
  expect(Boolean(view.container.querySelector('pre'))).toBe(false);
  expect(view.container.textContent).not.toContain('PRIVATE');
  expect(read).not.toHaveBeenCalled();
  expect(JSON.stringify(item)).toBe(before);
});

test('mixed reads, writes and patches share a file list with unique targets and only proven patch counts', () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockRejectedValue(new Error('File payload access must remain unused'));
  restore = () => read.mockRestore();
  const item = fileRun([
    file('read-page-one', 'read', [{ path: '/workspace/src/a file.ts' }]),
    file('read-page-two', 'read', [{ path: '/workspace/src/a file.ts' }]),
    file('write-new', 'write', [{ path: '/workspace/src/new file.ts' }]),
    file('patch-call', 'patch', [{ path: '/workspace/src/app.ts', added_lines: 8, removed_lines: 3 }]),
  ]);
  const before = JSON.stringify(item);
  const view = show(item);
  const toggle = view.getByRole('button', { name: 'Read 1 file, Edited 2 files' });
  expect(view.queryByRole('list')).toBeNull();
  fireEvent.click(toggle);
  const entries = view.getAllByRole('listitem');
  expect(entries).toHaveLength(3);
  expect(entries.map(entry => entry.textContent)).toEqual(['Read src/a file.ts', 'Edited src/new file.ts', 'Edited src/app.ts+8 −3']);
  expect(view.container.querySelectorAll('.turn-file-activity__changes')).toHaveLength(1);
  expect(Boolean(view.container.querySelector('pre'))).toBe(false);
  expect(read).not.toHaveBeenCalled();
  expect(JSON.stringify(item)).toBe(before);
});

test('a path edited by a write and patch keeps one target without inventing a complete line total', () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockRejectedValue(new Error('File payload access must remain unused'));
  restore = () => read.mockRestore();
  const view = show(fileRun([
    file('write', 'write', [{ path: '/workspace/src/app.ts' }]),
    file('patch', 'patch', [{ path: '/workspace/src/app.ts', added_lines: 8, removed_lines: 3 }]),
  ]));
  fireEvent.click(view.getByRole('button', { name: 'Edited 1 file' }));
  expect(view.getAllByRole('listitem')).toHaveLength(1);
  expect(view.getByRole('listitem').textContent).toContain('src/app.ts');
  expect(Boolean(view.container.querySelector('.turn-file-activity__changes'))).toBe(false);
  expect(read).not.toHaveBeenCalled();
});

test('failed file operations show targets and the public error without applied-change claims or payload requests', () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockRejectedValue(new Error('File payload access must remain unused'));
  restore = () => read.mockRestore();
  const view = show(file('failed-patch', 'patch', [{ path: '/workspace/src/app.ts', added_lines: 8, removed_lines: 3 }],
    { status: 'error', error: 'The requested patch was rejected.' }));
  fireEvent.click(view.getByRole('button'));
  expect(view.getByRole('listitem').textContent).toBe('Target src/app.ts');
  expect(view.container.textContent).toContain('The requested patch was rejected.');
  expect(Boolean(view.container.querySelector('.turn-file-activity__changes'))).toBe(false);
  expect(Boolean(view.container.querySelector('[data-tool-detail-section]'))).toBe(false);
  expect(read).not.toHaveBeenCalled();
});

test('file names without verified target metadata do not regain a raw payload inspector', () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockRejectedValue(new Error('File payload access must remain unused'));
  restore = () => read.mockRestore();
  const view = show(tool({ name: 'read_file', description: '/workspace/src/app.ts' }));
  expect(view.container.textContent).toContain('app.ts');
  expect(view.queryByRole('button')).toBeNull();
  expect(Boolean(view.container.querySelector('pre'))).toBe(false);
  expect(read).not.toHaveBeenCalled();
});

test('native failed completion reports show only the public error, without repeated summary or a payload request', () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockRejectedValue(new Error('Completion payload access must remain unused'));
  restore = () => read.mockRestore();
  const view = show(tool({ name: 'report_completion', status: 'error', error: 'Completion review rejected.', description: 'DUPLICATE FINAL SUMMARY' }));
  fireEvent.click(view.getByRole('button'));
  expect(view.container.textContent).toContain('Completion review rejected.');
  expect(view.container.textContent).not.toContain('DUPLICATE FINAL SUMMARY');
  expect(Boolean(view.container.querySelector('[data-tool-detail-section]'))).toBe(false);
  expect(read).not.toHaveBeenCalled();
});

test('a typed external tool sharing the completion name retains its original arguments and output inspector', async () => {
  const read = spyOn(database.getToolDetails, 'invoke').mockImplementation(async request => page(request,
    { format: 'json', content: request.section === 'arguments' ? '{"value":"ordinary input"}' : '{"value":"ordinary result"}' }));
  restore = () => read.mockRestore();
  const view = show(tool({ name: 'report_completion', capability_id: 'external', action_id: 'external/report' }));
  fireEvent.click(view.getByRole('button'));
  await waitFor(() => expect(view.container.textContent).toContain('ordinary result'));
  expect(view.container.textContent).toContain('ordinary input');
  expect(view.container.querySelectorAll('[data-tool-detail-section]')).toHaveLength(2);
  expect(read).toHaveBeenCalledTimes(2);
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
  expect(read).toHaveBeenCalledTimes(2);
  expect(view.container.querySelectorAll('[data-tool-detail-section]')).toHaveLength(2);
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
