import { afterEach, beforeEach, describe, expect, test } from 'bun:test';
import { act } from 'react';
import { cleanup, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { keyboard, type KeyboardActionRequest, type KeyboardEditFlush, type KeyboardState } from '@/common/keyboard';
import { useKeyboard } from '@/renderer/hooks/keyboard/useKeyboard';
import { useSubtitleEditor } from '@/renderer/hooks/keyboard/useSubtitleEditor';
import en from '@/renderer/services/i18n/locales/en-US/settings.json';
import KeyboardSubtitle from './KeyboardSubtitle';
import { subtitleTextMap } from './subtitleText';

const snapshot = (patch: Partial<KeyboardState> = {}): KeyboardState => ({
  sequence: 0, cancelGeneration: 0,
  settings: { enabled: true, k2Enabled: true, k3Enabled: true, k4Enabled: true, audioDeviceId: null, asrMode: 'streaming',
    asrModel: null, textModel: null, language: '', dictionary: [], beautifyInstruction: '', alwaysOnTop: true, subtitleFontSize: 14 },
  documentId: 'document-1', revision: 3, operationId: null, editSessionId: null, dictationSelection: null, phase: 'idle', text: '原文 👋\nSecond line',
  originalText: '原文 👋\nSecond line', partial: '', candidate: '', visible: true, pendingAction: null,
  error: null, notice: null,
  audioState: { state: 'stopped', detail: null },
  capturePreparing: false,
  device: { connected: true, id: 'device-1', port: 'COM8', firmware: 'test',
    policyState: 'applied', captureState: 'stopped', detail: null },
  service: { running: true, ready: true, detail: null }, ...patch,
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
const original = { ...keyboard };
const originalHasFocus = document.hasFocus;
const originalRects = Object.getOwnPropertyDescriptor(Range.prototype, 'getClientRects');
let current: KeyboardState;
let listener: ((state: KeyboardState) => void) | undefined;
let flushListener: ((request: KeyboardEditFlush) => void) | undefined;
let actions: KeyboardActionRequest[];
beforeEach(() => {
  current = snapshot(); actions = []; listener = undefined; flushListener = undefined;
  keyboard.available = () => true;
  keyboard.getState = async () => current;
  keyboard.listen = async (callback) => { listener = callback; return () => { listener = undefined; }; };
  keyboard.listenEditFlush = async (callback) => { flushListener = callback; return () => { flushListener = undefined; }; };
  keyboard.action = async (request) => { actions.push(request); return current; };
  keyboard.updateSettings = async (settings) => ({ ...current, settings });
});
afterEach(() => {
  cleanup(); Object.assign(keyboard, original); document.hasFocus = originalHasFocus;
  if (originalRects) Object.defineProperty(Range.prototype, 'getClientRects', originalRects);
  else Reflect.deleteProperty(Range.prototype, 'getClientRects');
});

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { settings: { keyboard: en.keyboard } } } } });
const renderBoard = () => render(<I18nextProvider i18n={i18n}><KeyboardSubtitle /></I18nextProvider>);
function applyEdit(request: KeyboardActionRequest) {
  current = { ...current, sequence: current.sequence + 1 };
  if (request.action === 'edit_begin') current = { ...current, editSessionId: request.editId! };
  if (request.action === 'edit') current = { ...current, text: request.text!, originalText: request.text!, revision: current.revision + 1 };
  if (request.action === 'edit_end' || request.action === 'edit_cancel') current = { ...current, editSessionId: null };
  if (request.action === 'close') current = { ...current, visible: false, text: '', originalText: '', editSessionId: null };
  listener?.(current);
  return current;
}
function editBackend() {
  keyboard.action = async request => { actions.push(request); return applyEdit(request); };
}
async function focusEditor() {
  await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
  const node = screen.getByRole('textbox', { name: 'Final text' });
  act(() => node.focus());
  await waitFor(() => expect(node.getAttribute('contenteditable')).toBe('plaintext-only'));
  return node;
}
function type(node: HTMLElement, text: string) { node.textContent = text; fireEvent.input(node); }
function select(node: HTMLElement, anchor: number, focus = anchor) {
  const map = subtitleTextMap(node); const start = map.locate(anchor); const end = map.locate(focus);
  window.getSelection()!.setBaseAndExtent(start[0], start[1], end[0], end[1]); fireEvent.mouseUp(node);
}
function publish(patch: Partial<KeyboardState>) { current = { ...current, ...patch, sequence: current.sequence + 1 }; listener?.(current); }
async function beginDictation(anchor: number, focus = anchor) {
  document.hasFocus = () => true;
  Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [new DOMRect(40, 30, 0, 28)] });
  keyboard.action = async request => {
    actions.push(request); applyEdit(request);
    if (request.action === 'edit_end') publish({ phase: 'recording', audioState: { state: 'capturing', detail: null }, dictationSelection: request.selection ?? null });
    return current;
  };
  renderBoard(); const node = await focusEditor();
  document.querySelector<HTMLElement>('.keyboard-subtitle__content')!.getBoundingClientRect = () => new DOMRect(0, 0, 720, 112);
  node.parentElement!.getBoundingClientRect = () => new DOMRect(0, 0, 720, 80);
  select(node, anchor, focus);
  act(() => flushListener?.({ documentId: current.documentId!, editId: current.editSessionId! }));
  await waitFor(() => expect(node.getAttribute('contenteditable')).toBe('false'));
  return node;
}

describe('native subtitle coordination', () => {
  for (const modifier of ['ctrlKey', 'metaKey'] as const) {
    test(`${modifier} selection copy retains the WebView default and never requests a native action`, async () => {
      editBackend(); renderBoard();
      const node = await focusEditor();
      select(node, 0, 2);
      const before = [...actions];
      expect(fireEvent.keyDown(node, { key: 'c', [modifier]: true })).toBe(true);
      expect(window.getSelection()!.toString()).toBe('原文');
      expect(actions).toEqual(before);
      expect(current.visible).toBe(true);
      expect(screen.queryByRole('button', { name: /copy/i })).toBeNull();
    });
  }

  test('Escape uses the close path once while pending and discards the current editor draft', async () => {
    document.hasFocus = () => true;
    const closed = deferred<KeyboardState>();
    keyboard.action = async request => {
      actions.push(request);
      return request.action === 'close' ? closed.promise : applyEdit(request);
    };
    renderBoard(); const node = await focusEditor();
    expect(fireEvent.keyDown(node, { key: 'Escape', repeat: true })).toBe(true);
    expect(fireEvent.keyDown(node, { key: 'Escape' })).toBe(false);
    fireEvent.keyDown(node, { key: 'Escape' });
    await waitFor(() => expect(actions.filter(value => value.action === 'close')).toEqual([
      { action: 'close', documentId: 'document-1', revision: 3, cancelGeneration: 0 },
    ]));
    expect(node.getAttribute('contenteditable')).toBe('false');
    await act(async () => { closed.resolve(applyEdit(actions.at(-1)!)); await closed.promise; });
    expect(current.visible).toBe(false);
    expect(current.text).toBe('');
  });

  test('Escape belongs to an active IME composition before it can close the subtitle', async () => {
    document.hasFocus = () => true;
    editBackend(); renderBoard(); const node = await focusEditor();
    fireEvent.compositionStart(node); type(node, '正在输入');
    expect(fireEvent.keyDown(node, { key: 'Escape' })).toBe(true);
    expect(actions.some(value => value.action === 'close')).toBe(false);
    fireEvent.compositionEnd(node);
    expect(fireEvent.keyDown(node, { key: 'Escape', isComposing: true })).toBe(true);
    expect(fireEvent.keyDown(node, { key: 'Escape', keyCode: 229 })).toBe(true);
    expect(actions.some(value => value.action === 'close')).toBe(false);
    fireEvent.keyDown(node, { key: 'Escape' });
    await waitFor(() => expect(actions.filter(value => value.action === 'close')).toHaveLength(1));
  });

  test('Escape ignores an unfocused subtitle window even when its body receives the event', async () => {
    editBackend(); renderBoard(); const node = await focusEditor();
    document.hasFocus = () => false;
    expect(fireEvent.keyDown(node, { key: 'Escape' })).toBe(true);
    const outside = document.createElement('input'); document.body.append(outside);
    try {
      await act(async () => { outside.focus(); await Promise.resolve(); });
      expect(fireEvent.keyDown(outside, { key: 'Escape' })).toBe(true);
      fireEvent.keyDown(document, { key: 'Escape' });
      expect(actions.some(value => value.action === 'close')).toBe(false);
    } finally { outside.remove(); }
  });

  test('Escape closes the focused subtitle window when its blank background or body owns focus', async () => {
    document.hasFocus = () => true;
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    expect(document.activeElement).toBe(document.body);
    expect(fireEvent.keyDown(document.body, { key: 'Escape' })).toBe(false);
    await waitFor(() => expect(actions).toEqual([{ action: 'close', documentId: 'document-1', revision: 3, cancelGeneration: 0 }]));
  });

  test('unmount removes the subtitle-local Escape listener before another route can use it', async () => {
    document.hasFocus = () => true;
    const board = renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    board.unmount();
    expect(fireEvent.keyDown(document.body, { key: 'Escape' })).toBe(true);
    expect(actions).toHaveLength(0);
  });

  test('Escape can close active capture before any finalized text exists', async () => {
    document.hasFocus = () => true;
    current = snapshot({ text: '', originalText: '', phase: 'recording', operationId: 'capture-1',
      audioState: { state: 'capturing', detail: null } });
    renderBoard(); await screen.findByText('Listening…');
    const node = screen.getByRole('textbox', { name: 'Final text' });
    act(() => node.focus()); fireEvent.keyDown(node, { key: 'Escape' });
    await waitFor(() => expect(actions).toEqual([{ action: 'close', documentId: 'document-1', revision: 3, cancelGeneration: 0 }]));
  });

  test('an old Escape reply cannot close or replace a newly opened document', async () => {
    document.hasFocus = () => true;
    const closed = deferred<KeyboardState>();
    keyboard.action = async request => { actions.push(request); return request.action === 'close' ? closed.promise : applyEdit(request); };
    renderBoard(); const node = await focusEditor();
    const old = { ...current, sequence: current.sequence + 1, visible: false, text: '', originalText: '' };
    fireEvent.keyDown(node, { key: 'Escape' });
    act(() => publish({ sequence: old.sequence, documentId: 'document-2', text: 'new document', editSessionId: null }));
    await screen.findByText('new document');
    await act(async () => { closed.resolve(old); await closed.promise; });
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('new document');
    expect(current.documentId).toBe('document-2');
    expect(current.visible).toBe(true);
    expect(actions.filter(value => value.action === 'close')).toHaveLength(1);
  });

  test('cancelling capture preparation retains an unsaved editor draft and its lease', async () => {
    const save = deferred<KeyboardState>();
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'edit') return save.promise;
      if (request.action === 'cancel') { publish({ capturePreparing: false }); return current; }
      return applyEdit(request);
    };
    renderBoard(); const node = await focusEditor();
    type(node, 'keep this local draft'); select(node, 4);
    act(() => publish({ capturePreparing: true }));
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    expect(node.textContent).toBe('keep this local draft');
    expect(node.getAttribute('contenteditable')).toBe('plaintext-only');
    expect(window.getSelection()!.anchorOffset).toBe(4);
    expect(actions.some(value => value.action === 'edit_cancel' || value.action === 'edit_end')).toBe(false);
    await act(async () => { save.resolve(applyEdit(actions.find(value => value.action === 'edit')!)); await save.promise; });
    await waitFor(() => expect(current.text).toBe('keep this local draft'));
  });

  test('a late action reply cannot replace a newer native document event', async () => {
    const response = deferred<KeyboardState>();
    keyboard.action = () => response.promise;
    const hook = renderHook(useKeyboard);
    await waitFor(() => expect(hook.result.current.state?.revision).toBe(3));
    let result: Promise<KeyboardState | null> | undefined;
    act(() => { result = hook.result.current.action({ action: 'beautify', documentId: 'document-1', revision: 3 }); });
    act(() => listener?.(snapshot({ documentId: 'document-2', revision: 0, text: 'new document' })));
    await act(async () => { response.resolve(snapshot({ revision: 4, text: 'late candidate' })); await result; });
    expect(hook.result.current.state?.documentId).toBe('document-2');
    expect(hook.result.current.state?.text).toBe('new document');
  });

  test('unmount during event registration disposes the late subscription without a state read', async () => {
    const subscription = deferred<() => void>();
    let stopped = false;
    let reads = 0;
    keyboard.listen = () => subscription.promise;
    keyboard.getState = async () => { reads++; return current; };
    const hook = renderHook(useKeyboard);
    hook.unmount();
    await act(async () => { subscription.resolve(() => { stopped = true; }); await subscription.promise; });
    expect(stopped).toBe(true);
    expect(reads).toBe(0);
  });

  test('a higher-sequence reply survives an intervening old tick and lower-sequence events are ignored', async () => {
    current = snapshot({ sequence: 10 });
    const response = deferred<KeyboardState>(); keyboard.action = () => response.promise;
    const hook = renderHook(useKeyboard);
    await waitFor(() => expect(hook.result.current.state?.sequence).toBe(10));
    let result: Promise<KeyboardState | null> | undefined;
    act(() => { result = hook.result.current.action({ action: 'edit', documentId: 'document-1', revision: 3, editId: 'lease', text: 'saved' }); });
    act(() => listener?.(snapshot({ sequence: 11 })));
    await act(async () => { response.resolve(snapshot({ sequence: 12, revision: 4, text: 'saved' })); await result; });
    expect(hook.result.current.state?.text).toBe('saved');
    act(() => listener?.(snapshot({ sequence: 11, text: 'old tick' })));
    expect(hook.result.current.state?.sequence).toBe(12);
    expect(hook.result.current.getCurrentState()?.text).toBe('saved');
  });

  test('recording can be cancelled with empty text and every document action carries identity', async () => {
    current = snapshot({ phase: 'recording', text: '', partial: '临时字幕', operationId: 'recording-1' });
    renderBoard();
    const cancel = await screen.findByRole('button', { name: 'Cancel' });
    fireEvent.click(cancel);
    await waitFor(() => expect(actions).toContainEqual({ action: 'cancel', documentId: 'document-1', revision: 3, cancelGeneration: 0 }));
    fireEvent.click(screen.getByRole('button', { name: 'Close subtitle board' }));
    await waitFor(() => expect(actions).toContainEqual({ action: 'close', documentId: 'document-1', revision: 3, cancelGeneration: 0 }));
    expect(screen.getByText('临时字幕')).toBeTruthy();
  });

  test('cancelling beautification restores committed text and never revives its unfinished preview', async () => {
    current = snapshot({ phase: 'beautifying', candidate: 'unfinished candidate', operationId: 'beautify-1' });
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'cancel') publish({ phase: 'idle', operationId: null, cancelGeneration: 1 });
      return current;
    };
    renderBoard();
    await screen.findByText('unfinished candidate');
    const stale = { ...current, sequence: current.sequence - 1, candidate: 'late candidate' };
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(actions).toEqual([{ action: 'cancel', documentId: 'document-1', revision: 3, cancelGeneration: 0 }]));
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('原文 👋\nSecond line');
    act(() => listener?.(stale));
    expect(screen.queryByText('unfinished candidate')).toBeNull();
    expect(screen.queryByText('late candidate')).toBeNull();
    expect(current.text).toBe('原文 👋\nSecond line');
    expect(current.revision).toBe(3);
  });

  test('close failure leaves final text and close controls available', async () => {
    keyboard.action = async () => { throw new Error('Cannot stop recording'); };
    renderBoard();
    const close = await screen.findByRole('button', { name: 'Close subtitle board' });
    await waitFor(() => expect(close.hasAttribute('disabled')).toBe(false));
    fireEvent.click(close);
    await screen.findByText('Cannot stop recording');
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('原文 👋\nSecond line');
    expect(close.hasAttribute('disabled')).toBe(false);
  });

  test('recognition failure is a compact independent status with optional details, never caption text', async () => {
    const diagnostic = 'Recognition interrupted; existing text is kept. Captured 1.9 s, peak -54 dBFS.';
    current = snapshot({ error: diagnostic });
    renderBoard();
    const issue = await screen.findByRole('alert');
    expect(issue.textContent).toBe('Operation incomplete · Show details');
    expect(issue.textContent).not.toContain('dBFS');
    const details = issue.closest('details')!;
    expect(details.hasAttribute('open')).toBe(false);
    expect(details.textContent).toContain(diagnostic);
    expect(document.querySelector('.keyboard-subtitle__content')!.contains(details)).toBe(false);
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text);
    expect(screen.getByRole('button', { name: 'Beautify' }).hasAttribute('disabled')).toBe(false);
  });

  test('a fresh capture keeps the listening indicator clear of an earlier issue detail', async () => {
    current = snapshot({ phase: 'recording', error: 'Previous operation interrupted', audioState: { state: 'capturing', detail: null } });
    renderBoard();
    await screen.findByText('Listening…');
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text);
  });

  test('pinning uses only the subtitle action capability with current document identity', async () => {
    renderBoard();
    const pin = await screen.findByRole('button', { name: 'Keep subtitle window on top' });
    await waitFor(() => expect(pin.hasAttribute('disabled')).toBe(false));
    fireEvent.click(pin);
    await waitFor(() => expect(actions).toEqual([{ action: 'toggle_pin', documentId: 'document-1', revision: 3, cancelGeneration: 0 }]));
  });

  test('standard ASR announces recording until release and never presents interim streaming text', async () => {
    current = snapshot({ phase: 'recording', text: '', partial: 'not a live transcript', audioState: { state: 'capturing', detail: null } });
    current.settings.asrMode = 'platform';
    renderBoard();
    await screen.findByText('Recording; transcription starts after release…');
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('');
    expect(screen.queryByText('not a live transcript')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(actions).toEqual([{ action: 'cancel', documentId: 'document-1', revision: 3, cancelGeneration: 0 }]));
  });

  test('standard ASR finalization reports recognition and allows cancellation', async () => {
    current = snapshot({ phase: 'finalizing', text: '', operationId: 'upload-1' });
    current.settings.asrMode = 'platform';
    renderBoard();
    await screen.findByText('Uploading and recognizing this recording…');
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('Recognizing…');
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(actions).toEqual([{ action: 'cancel', documentId: 'document-1', revision: 3, cancelGeneration: 0 }]));
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).not.toContain('not a live transcript');
  });

  test('an idle pending capture shows connection progress without claiming recording or taking focus', async () => {
    current = snapshot({ documentId: null, revision: 0, phase: 'idle', text: '', originalText: '',
      audioState: { state: 'starting', detail: null } });
    const outside = document.createElement('input');
    document.body.append(outside); outside.focus();
    try {
      renderBoard();
      await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(''));
      expect(document.activeElement).toBe(outside);
      expect(screen.getByRole('status').textContent).toBe('Preparing…');
      expect(screen.queryByText('Recording…')).toBeNull();
      fireEvent.click(screen.getByRole('button', { name: 'Stop recording' }));
      await waitFor(() => expect(actions).toEqual([{ action: 'stop', documentId: null, revision: 0, cancelGeneration: 0 }]));
      act(() => listener?.(snapshot({ documentId: null, revision: 0, text: '', originalText: '' })));
      await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(''));
      expect(document.activeElement).toBe(outside);
    } finally { outside.remove(); }
  });

  test('waiting for recognition keeps the complete current document visible', async () => {
    current = snapshot({ phase: 'idle', audioState: { state: 'starting', detail: null } });
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    expect(screen.getByRole('status').textContent).toBe('Preparing…');
    expect(document.querySelector('.keyboard-subtitle__empty')).toBeNull();
    act(() => listener?.(snapshot({ phase: 'recording', audioState: { state: 'capturing', detail: null }, text: '', originalText: '' })));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(''));
  });

  test('lyrics stream one glowing candidate without mutating the committed document', async () => {
    current = snapshot({ phase: 'beautifying', operationId: 'beautify-1' });
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    const text = screen.getByRole('textbox', { name: 'Final text' });
    expect(text.getAttribute('aria-busy')).toBe('true');
    expect(screen.getByRole('main').classList.contains('keyboard-subtitle--beautifying')).toBe(true);
    act(() => publish({ candidate: 'first streamed words' }));
    expect(text.textContent).toBe('first streamed words');
    expect(text.getAttribute('contenteditable')).toBe('false');
    expect(current.text).toBe('原文 👋\nSecond line');
    expect(current.revision).toBe(3);
    expect(current.originalText).toBe('原文 👋\nSecond line');
    act(() => publish({ candidate: 'first streamed words, now complete.' }));
    expect(text.textContent).toBe('first streamed words, now complete.');
    expect(current.text).toBe('原文 👋\nSecond line');
    fireEvent.input(text);
    expect(actions).toHaveLength(0);
    act(() => publish({ phase: 'idle', operationId: null, text: 'Final completed text.', revision: 4, candidate: '' }));
    expect(text.textContent).toBe('Final completed text.');
    expect(current.text).toBe('Final completed text.');
    expect(text.getAttribute('aria-busy')).toBe('false');
    expect(screen.getByRole('main').classList.contains('keyboard-subtitle--beautifying')).toBe(false);
    expect(screen.queryByRole('heading')).toBeNull();
    expect(screen.queryByText('NomiFun subtitle board')).toBeNull();
  });

  test('failed or truncated beautification discards the preview and retains the original revision', async () => {
    current = snapshot({ phase: 'beautifying', candidate: 'a promising partial rewrite', operationId: 'beautify-1' });
    renderBoard(); await screen.findByText('a promising partial rewrite');
    act(() => publish({ phase: 'idle', operationId: null, error: 'The response was truncated.' }));
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('原文 👋\nSecond line');
    expect(current.text).toBe('原文 👋\nSecond line'); expect(current.revision).toBe(3);
    act(() => publish({ candidate: 'late preview after failure' }));
    expect(screen.queryByText('late preview after failure')).toBeNull();
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text);
    expect(actions).toHaveLength(0);
  });

  test('streaming a polish preview leaves an existing external focus alone', async () => {
    const outside = document.createElement('input'); document.body.append(outside); outside.focus();
    try {
      current = snapshot({ phase: 'beautifying', operationId: 'beautify-1' });
      renderBoard();
      await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
      act(() => publish({ candidate: 'visible while the model is generating' }));
      expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.candidate);
      expect(document.activeElement).toBe(outside);
      expect(document.querySelector('.keyboard-subtitle__caret')).toBeNull();
      expect(actions).toHaveLength(0);
    } finally { outside.remove(); }
  });

  test('candidate events leave the focused editor and its in-progress IME draft untouched', async () => {
    editBackend(); renderBoard(); const node = await focusEditor();
    fireEvent.compositionStart(node); type(node, '正在编辑的草稿');
    select(node, 3);
    act(() => publish({ candidate: 'candidate must never become the editing source' }));
    expect(node.textContent).toBe('正在编辑的草稿');
    expect(document.activeElement).toBe(node);
    expect(node.getAttribute('contenteditable')).toBe('plaintext-only');
    expect(window.getSelection()!.anchorOffset).toBe(3);
    expect(actions.map(value => value.action)).toEqual(['edit_begin']);
    fireEvent.compositionEnd(node);
    await waitFor(() => expect(current.text).toBe('正在编辑的草稿'));
    expect(actions.filter(value => value.action === 'edit').map(value => value.text)).toEqual(['正在编辑的草稿']);
    expect(screen.queryByText('candidate must never become the editing source')).toBeNull();
  });

  test('mount and native text updates leave an existing DOM focus alone', async () => {
    const outside = document.createElement('input');
    document.body.append(outside); outside.focus();
    try {
      renderBoard();
      await screen.findByRole('textbox', { name: 'Final text' });
      expect(document.activeElement).toBe(outside);
      act(() => listener?.(snapshot({ text: 'Updated final text', revision: 4 })));
      await screen.findByText('Updated final text');
      expect(document.activeElement).toBe(outside);
      expect(document.querySelector('[autofocus]')).toBeNull();
    } finally { outside.remove(); }
  });

  test('compact lyrics retain the complete selectable long document', async () => {
    const text = '完整正文 👋\n'.repeat(300);
    current = snapshot({ text });
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(text));
    const body = screen.getByRole('textbox', { name: 'Final text' });
    expect(body.getAttribute('tabindex')).toBe('0');
    expect(screen.getByRole('button', { name: 'Drag subtitle window' }).hasAttribute('data-tauri-drag-region')).toBe(true);
  });

  test('idle empty lyrics stay visually blank while retaining an accessible status', async () => {
    current = snapshot({ text: '', partial: '' });
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(''));
    expect(screen.getByRole('status').textContent).toBe('Hold K4 to speak');
  });

  test('an operation notice stays outside completed lyrics', async () => {
    current = snapshot({ notice: 'Operation cancelled' });
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    expect(screen.getByRole('status').textContent).toContain('Operation cancelled');
  });

  test('an empty-document notice can be the compact placeholder', async () => {
    current = snapshot({ text: '', notice: 'No speech detected' });
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('No speech detected'));
  });

  test('a new document gets a fresh lyric surface after the previous model operation', async () => {
    current = snapshot({ phase: 'beautifying' });
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    const previous = screen.getByRole('textbox', { name: 'Final text' });
    act(() => listener?.(snapshot({ documentId: 'document-2', text: 'New document', revision: 0 })));
    await screen.findByText('New document');
    expect(screen.getByRole('textbox', { name: 'Final text' })).not.toBe(previous);
    expect(screen.getByRole('main').classList.contains('keyboard-subtitle--beautifying')).toBe(false);
  });

  test('editing waits for its native lease and never focuses the subtitle automatically', async () => {
    const begin = deferred<KeyboardState>();
    keyboard.action = request => { actions.push(request); return begin.promise; };
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    const node = screen.getByRole('textbox', { name: 'Final text' });
    expect(node.getAttribute('contenteditable')).toBe('false');
    expect(actions).toHaveLength(0);
    act(() => node.focus());
    await waitFor(() => expect(actions[0]?.action).toBe('edit_begin'));
    expect(node.getAttribute('contenteditable')).toBe('false');
    await act(async () => { const saved = applyEdit(actions[0]); begin.resolve(saved); await begin.promise; });
    await waitFor(() => expect(node.getAttribute('contenteditable')).toBe('plaintext-only'));
  });

  test('only explicit focus cancels an unknown lease before acquiring a new one', async () => {
    current = snapshot({ editSessionId: 'recreated-window-lease' }); editBackend(); renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    expect(actions).toHaveLength(0);
    await focusEditor();
    expect(actions.map(value => value.action)).toEqual(['edit_cancel', 'edit_begin']);
    expect(actions[0].editId).toBe('recreated-window-lease');
    expect(actions[1].editId).not.toBe('recreated-window-lease');
  });

  test('closing while a lease begins prevents its late reply from enabling editing', async () => {
    const pending = deferred<KeyboardState>();
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'edit_begin') return pending.promise;
      if (request.action === 'close') current = { ...current, visible: false, sequence: current.sequence + 2 };
      listener?.(current); return current;
    };
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    const node = screen.getByRole('textbox', { name: 'Final text' });
    act(() => node.focus()); await waitFor(() => expect(actions[0]?.action).toBe('edit_begin'));
    fireEvent.click(screen.getByRole('button', { name: 'Close subtitle board' }));
    await act(async () => { pending.resolve(snapshot({ sequence: 1, editSessionId: actions[0].editId! })); await pending.promise; });
    expect(node.getAttribute('contenteditable')).toBe('false');
    expect(actions.filter(value => value.action === 'edit_begin')).toHaveLength(1);
  });

  test('inputs save in one flight and an older acknowledgement leaves the newer DOM draft and caret intact', async () => {
    const first = deferred<KeyboardState>();
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'edit' && actions.filter(value => value.action === 'edit').length === 1) return first.promise;
      return applyEdit(request);
    };
    renderBoard();
    const node = await focusEditor();
    type(node, 'first 👋\nline');
    type(node, 'latest 你好🙂\nnext');
    const textNode = node.firstChild!;
    const range = document.createRange(); range.setStart(textNode, 3); range.collapse(true);
    window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range);
    act(() => listener?.({ ...current, service: { ...current.service, detail: 'status event' } }));
    expect(node.textContent).toBe('latest 你好🙂\nnext');
    expect(actions.filter(value => value.action === 'edit')).toHaveLength(1);
    await act(async () => { first.resolve(applyEdit(actions.find(value => value.action === 'edit')!)); await first.promise; });
    await waitFor(() => expect(current.text).toBe('latest 你好🙂\nnext'));
    expect(actions.filter(value => value.action === 'edit').map(value => value.revision)).toEqual([3, 4]);
    expect(node.firstChild).toBe(textNode);
    expect(window.getSelection()!.anchorOffset).toBe(3);
  });

  test('a hardware flush waits for IME completion and saves the composed text before ending the lease', async () => {
    editBackend(); renderBoard();
    const node = await focusEditor();
    fireEvent.compositionStart(node);
    type(node, '正在输入中文🙂');
    act(() => flushListener?.({ documentId: current.documentId!, editId: current.editSessionId! }));
    await act(async () => { await Promise.resolve(); });
    expect(actions.map(value => value.action)).toEqual(['edit_begin']);
    fireEvent.compositionEnd(node);
    await waitFor(() => expect(actions.filter(value => value.action !== 'edit_begin').map(value => value.action)).toEqual(['edit', 'edit_end']));
    expect(actions[1].text).toBe('正在输入中文🙂');
    expect(actions[2].revision).toBe(4);
  });

  test('beautify flushes edits and ends the lease before using the resulting document revision', async () => {
    const first = deferred<KeyboardState>();
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'edit') return first.promise;
      return applyEdit(request);
    };
    renderBoard(); const node = await focusEditor(); type(node, 'edited result');
    fireEvent.click(screen.getByRole('button', { name: 'Beautify' }));
    expect(actions.some(value => value.action === 'beautify')).toBe(false);
    await act(async () => { first.resolve(applyEdit(actions.find(value => value.action === 'edit')!)); await first.promise; });
    await waitFor(() => expect(actions.some(value => value.action === 'beautify')).toBe(true));
    expect(actions.map(value => value.action)).toEqual(['edit_begin', 'edit', 'edit_end', 'beautify']);
    expect(actions[3]).toEqual({ action: 'beautify', documentId: 'document-1', revision: 4, cancelGeneration: 0 });
  });

  test('hardware cancellation during IME saves the composition without reviving the queued beautify', async () => {
    editBackend(); renderBoard(); const node = await focusEditor();
    fireEvent.compositionStart(node); type(node, '正在输入的草稿');
    fireEvent.click(screen.getByRole('button', { name: 'Beautify' }));
    expect(actions.map(value => value.action)).toEqual(['edit_begin']);
    act(() => publish({ cancelGeneration: 1 }));
    expect(node.textContent).toBe('正在输入的草稿');
    expect(node.getAttribute('contenteditable')).toBe('plaintext-only');
    fireEvent.compositionEnd(node);
    await waitFor(() => expect(actions.filter(value => value.action === 'edit_end')).toHaveLength(1));
    expect(current.text).toBe('正在输入的草稿');
    expect(actions.some(value => value.action === 'beautify')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Beautify' }));
    await waitFor(() => expect(actions.filter(value => value.action === 'beautify')).toEqual([
      { action: 'beautify', documentId: 'document-1', revision: 4, cancelGeneration: 1 },
    ]));
  });

  test('hardware cancellation during a slow edit preserves the save and fences the old UI intention', async () => {
    const save = deferred<KeyboardState>();
    keyboard.action = async request => {
      actions.push(request);
      return request.action === 'edit' ? save.promise : applyEdit(request);
    };
    renderBoard(); const node = await focusEditor(); type(node, 'keep the pending edit');
    fireEvent.click(screen.getByRole('button', { name: 'Beautify' }));
    act(() => publish({ cancelGeneration: 1 }));
    expect(node.textContent).toBe('keep the pending edit');
    expect(actions.some(value => value.action === 'edit_cancel')).toBe(false);
    await act(async () => { save.resolve(applyEdit(actions.find(value => value.action === 'edit')!)); await save.promise; });
    await waitFor(() => expect(actions.filter(value => value.action === 'edit_end')).toHaveLength(1));
    expect(current.text).toBe('keep the pending edit');
    expect(actions.some(value => value.action === 'beautify')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Beautify' }));
    await waitFor(() => expect(actions.filter(value => value.action === 'beautify')).toEqual([
      { action: 'beautify', documentId: 'document-1', revision: 4, cancelGeneration: 1 },
    ]));
  });

  test('the cancel control revokes a queued UI action before its native reply arrives', async () => {
    const save = deferred<KeyboardState>(); const cancelled = deferred<KeyboardState>();
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'edit') return save.promise;
      if (request.action === 'cancel') return cancelled.promise;
      return applyEdit(request);
    };
    renderBoard(); const node = await focusEditor(); type(node, 'keep this draft');
    fireEvent.click(screen.getByRole('button', { name: 'Beautify' }));
    act(() => publish({ capturePreparing: true }));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await act(async () => { save.resolve(applyEdit(actions.find(value => value.action === 'edit')!)); await save.promise; });
    await waitFor(() => expect(actions.filter(value => value.action === 'edit_end')).toHaveLength(1));
    expect(current.text).toBe('keep this draft');
    expect(actions.some(value => value.action === 'beautify')).toBe(false);
    await act(async () => {
      publish({ capturePreparing: false, cancelGeneration: 1 }); cancelled.resolve(current); await cancelled.promise;
    });
    expect(actions.some(value => value.action === 'beautify')).toBe(false);
  });

  test('a new action already using the native cancellation generation survives its renderer catch-up', async () => {
    let native = snapshot();
    const runs: number[] = [];
    const hook = renderHook(({ state }) => useSubtitleEditor(state, async () => native, () => native, () => false, () => null),
      { initialProps: { state: native } });
    native = { ...native, cancelGeneration: 1 };
    let requested: Promise<void> | undefined;
    act(() => {
      requested = hook.result.current.runAfterSave(async state => { runs.push(state.cancelGeneration); return native; });
      hook.rerender({ state: native });
    });
    await act(async () => { await requested; });
    expect(runs).toEqual([1]);
  });

  test('save failure retains the draft and never releases a hardware intent until explicit retry succeeds', async () => {
    let fail = true;
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'edit' && fail) throw new Error('save unavailable');
      return applyEdit(request);
    };
    renderBoard(); const node = await focusEditor(); type(node, 'unsaved draft');
    await screen.findByRole('button', { name: 'Retry saving edits' });
    act(() => flushListener?.({ documentId: current.documentId!, editId: current.editSessionId! }));
    await act(async () => { await Promise.resolve(); });
    expect(node.textContent).toBe('unsaved draft');
    expect(actions.some(value => value.action === 'edit_end')).toBe(false);
    fail = false; fireEvent.click(screen.getByRole('button', { name: 'Retry saving edits' }));
    await waitFor(() => expect(current.text).toBe('unsaved draft'));
    await waitFor(() => expect(actions.filter(value => value.action === 'edit_end')).toHaveLength(1));
  });

  test('switching documents revokes a waiting beautify and ignores the old edit reply', async () => {
    const pending = deferred<KeyboardState>();
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'edit') return pending.promise;
      return applyEdit(request);
    };
    renderBoard(); const node = await focusEditor(); type(node, 'old draft');
    const old = { ...current, text: 'old draft', originalText: 'old draft', revision: 4 };
    fireEvent.click(screen.getByRole('button', { name: 'Beautify' }));
    act(() => { current = snapshot({ sequence: current.sequence + 1, documentId: 'document-2', text: 'new document' }); listener?.(current); });
    await screen.findByText('new document');
    await act(async () => { pending.resolve(old); await pending.promise; });
    expect(actions.some(value => value.action === 'beautify' || value.action === 'edit_end')).toBe(false);
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('new document');
  });

  test('a previous document save cannot block or clear the new document save and beautify', async () => {
    const oldSave = deferred<KeyboardState>(); const newSave = deferred<KeyboardState>();
    let oldReply!: KeyboardState;
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'edit' && request.documentId === 'document-1') {
        oldReply = { ...current, text: request.text!, revision: request.revision! + 1, sequence: current.sequence + 1 };
        return oldSave.promise;
      }
      if (request.action === 'edit' && request.text === 'new initial') return newSave.promise;
      return applyEdit(request);
    };
    renderBoard(); type(await focusEditor(), 'old pending');
    fireEvent.click(screen.getByRole('button', { name: 'Beautify' }));
    act(() => publish({ documentId: 'document-2', revision: 0, text: 'new document', editSessionId: null }));
    const node = await focusEditor(); type(node, 'new initial');
    await waitFor(() => expect(actions.filter(value => value.action === 'edit' && value.documentId === 'document-2')).toHaveLength(1));
    fireEvent.click(screen.getByRole('button', { name: 'Beautify' }));
    await act(async () => { oldSave.resolve(oldReply); await oldSave.promise; });
    type(node, 'new latest');
    expect(actions.filter(value => value.action === 'edit' && value.documentId === 'document-2')).toHaveLength(1);
    await act(async () => {
      newSave.resolve(applyEdit(actions.find(value => value.action === 'edit' && value.documentId === 'document-2')!));
      await newSave.promise;
    });
    await waitFor(() => expect(actions.filter(value => value.action === 'beautify')).toEqual([
      { action: 'beautify', documentId: 'document-2', revision: 2, cancelGeneration: 0 },
    ]));
    expect(current.text).toBe('new latest');
    expect(actions.filter(value => value.action === 'edit_end' && value.documentId === 'document-1')).toHaveLength(0);
  });

  test('a pending lease for an older document does not block a fresh editor lease', async () => {
    const oldBegin = deferred<KeyboardState>(); let oldReply!: KeyboardState;
    keyboard.action = async request => {
      actions.push(request);
      if (request.action === 'edit_begin' && request.documentId === 'document-1') {
        oldReply = { ...current, editSessionId: request.editId!, sequence: current.sequence + 1 };
        return oldBegin.promise;
      }
      return applyEdit(request);
    };
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text));
    const oldNode = screen.getByRole('textbox', { name: 'Final text' });
    act(() => oldNode.focus());
    await waitFor(() => expect(actions[0]?.action).toBe('edit_begin'));
    act(() => publish({ documentId: 'document-2', revision: 0, text: 'new document', editSessionId: null }));
    const node = await focusEditor();
    await act(async () => { oldBegin.resolve(oldReply); await oldBegin.promise; });
    type(node, 'new draft');
    await waitFor(() => expect(current.text).toBe('new draft'));
    expect(actions.filter(value => value.action === 'edit_begin' && value.documentId === 'document-2')).toHaveLength(1);
    expect(node.getAttribute('contenteditable')).toBe('plaintext-only');
  });

  test('K4 keeps a visible collapsed caret, inserts interim text in place, and advances to the final insertion end', async () => {
    current = snapshot({ text: '甲🙂乙', originalText: '甲🙂乙' });
    const node = await beginDictation(3);
    expect(actions.find(value => value.action === 'edit_end')?.selection).toEqual({ anchor: 3, focus: 3 });
    await waitFor(() => expect(document.querySelector('.keyboard-subtitle__caret')).toBeTruthy());
    expect(document.activeElement).toBe(node);
    const prefix = node.firstChild;
    act(() => publish({ partial: '暂定' }));
    expect(node.textContent).toBe('甲🙂暂定乙');
    expect(node.firstChild).toBe(prefix);
    act(() => publish({ partial: '', text: '甲🙂新乙', originalText: '甲🙂新乙', revision: 4,
      phase: 'idle', audioState: { state: 'stopped', detail: null }, dictationSelection: { anchor: 4, focus: 4 } }));
    await waitFor(() => expect(node.getAttribute('contenteditable')).toBe('plaintext-only'));
    const range = window.getSelection()!; const map = subtitleTextMap(node);
    expect(map.offset(range.anchorNode!, range.anchorOffset)).toBe(4);
    expect(range.isCollapsed).toBe(true);
    expect(node.textContent).toBe('甲🙂新乙');
    expect(document.querySelector('.keyboard-subtitle__caret')).toBeNull();
  });

  test('a backward UTF-16 range is submitted and interim replacement has one visible body', async () => {
    current = snapshot({ text: '甲🙂乙丙', originalText: '甲🙂乙丙' });
    const node = await beginDictation(5, 1);
    expect(actions.find(value => value.action === 'edit_end')?.selection).toEqual({ anchor: 5, focus: 1 });
    expect(document.querySelector('.keyboard-subtitle__caret')).toBeNull();
    act(() => publish({ partial: '新词' }));
    expect(node.textContent).toBe('甲新词');
    act(() => publish({ partial: '', phase: 'idle', text: '甲新词', revision: 4,
      audioState: { state: 'stopped', detail: null }, dictationSelection: { anchor: 3, focus: 3 } }));
    await waitFor(() => expect(node.getAttribute('contenteditable')).toBe('plaintext-only'));
    const range = window.getSelection()!;
    expect(subtitleTextMap(node).offset(range.anchorNode!, range.anchorOffset)).toBe(3);
  });

  test('blur and external focus revoke the caret and cannot restore a lease after the recording finishes', async () => {
    const node = await beginDictation(1);
    await waitFor(() => expect(document.querySelector('.keyboard-subtitle__caret')).toBeTruthy());
    const count = actions.filter(value => value.action === 'edit_begin').length;
    const outside = document.createElement('input'); document.body.append(outside);
    try {
      act(() => { outside.focus(); window.dispatchEvent(new Event('blur')); });
      expect(document.querySelector('.keyboard-subtitle__caret')).toBeNull();
      act(() => publish({ phase: 'idle', text: 'new final', revision: 4, partial: '',
        audioState: { state: 'stopped', detail: null }, dictationSelection: { anchor: 2, focus: 2 } }));
      await waitFor(() => expect(node.textContent).toBe('new final'));
      expect(document.activeElement).toBe(outside);
      expect(actions.filter(value => value.action === 'edit_begin')).toHaveLength(count);
      expect(node.getAttribute('contenteditable')).toBe('false');
    } finally { outside.remove(); }
  });

  test('close and a different document never inherit the previous visible caret or selection', async () => {
    await beginDictation(1);
    await waitFor(() => expect(document.querySelector('.keyboard-subtitle__caret')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: 'Close subtitle board' }));
    expect(document.querySelector('.keyboard-subtitle__caret')).toBeNull();
    act(() => publish({ documentId: 'document-2', phase: 'idle', text: 'another document', revision: 0,
      editSessionId: null, partial: '', audioState: { state: 'stopped', detail: null }, dictationSelection: null }));
    await screen.findByText('another document');
    expect(document.querySelector('.keyboard-subtitle__caret')).toBeNull();
    expect(screen.getByRole('textbox', { name: 'Final text' }).getAttribute('contenteditable')).toBe('false');
  });

  test('an off-document selection prevents a late caret or editing lease restore without changing DOM focus', async () => {
    const node = await beginDictation(1);
    const count = actions.filter(value => value.action === 'edit_begin').length;
    const outside = document.createElement('span'); outside.textContent = 'outside'; document.body.append(outside);
    try {
      act(() => {
        window.getSelection()!.setBaseAndExtent(outside.firstChild!, 1, outside.firstChild!, 1);
        document.dispatchEvent(new Event('selectionchange'));
      });
      expect(document.activeElement).toBe(node);
      act(() => publish({ phase: 'idle', text: 'new result', partial: '', revision: 4,
        audioState: { state: 'stopped', detail: null }, dictationSelection: { anchor: 2, focus: 2 } }));
      await waitFor(() => expect(node.textContent).toBe('new result'));
      expect(window.getSelection()!.anchorNode).toBe(outside.firstChild);
      expect(document.querySelector('.keyboard-subtitle__caret')).toBeNull();
      expect(actions.filter(value => value.action === 'edit_begin')).toHaveLength(count);
    } finally { outside.remove(); }
  });

  for (const mode of ['streaming', 'platform'] as const) {
    test(`${mode} shows recording readiness above an existing document and removes listening motion as soon as stopping begins`, async () => {
      current = snapshot({ phase: 'recording', audioState: { state: 'starting', detail: null } });
      current.settings.asrMode = mode; renderBoard();
      await waitFor(() => expect(document.querySelector('.keyboard-subtitle__capture-status[data-state="preparing"]')).toBeTruthy());
      expect(document.querySelector('.keyboard-subtitle__capture-status')?.textContent).toBe('Preparing…');
      expect(screen.getAllByRole('status')).toHaveLength(1);
      expect(screen.getByRole('status').textContent).toBe('Preparing…');
      const node = screen.getByRole('textbox', { name: 'Final text' });
      expect(node.textContent).toBe(current.text);
      expect(document.querySelector('.keyboard-subtitle__listen-bars')).toBeNull();
      expect(screen.queryByText('Listening…')).toBeNull();
      act(() => publish({ audioState: { state: 'capturing', detail: null } }));
      await screen.findByText('Listening…');
      expect(document.querySelectorAll('.keyboard-subtitle__listen-bars i')).toHaveLength(5);
      const indicator = document.querySelector('.keyboard-subtitle__capture-status')!;
      expect(indicator.getAttribute('aria-hidden')).toBe('true');
      expect(node.contains(indicator)).toBe(false);
      expect(subtitleTextMap(node).text).toBe(current.text);
      const range = document.createRange(); range.selectNodeContents(node);
      expect(range.toString()).toBe(current.text);
      fireEvent.click(screen.getByRole('button', { name: 'Stop recording' }));
      await waitFor(() => expect(actions).toContainEqual({ action: 'stop', documentId: 'document-1', revision: 3, cancelGeneration: 0 }));
      expect(actions.at(-1)?.text).toBeUndefined();
      act(() => publish({ phase: 'finalizing', audioState: { state: 'stopping', detail: null } }));
      await waitFor(() => expect(document.querySelector('.keyboard-subtitle__capture-status[data-state="finishing"]')).toBeTruthy());
      expect(document.querySelector('.keyboard-subtitle__listen-bars')).toBeNull();
      expect(document.querySelector('.keyboard-subtitle__prepare-spinner')).toBeNull();
      act(() => publish({ audioState: { state: 'stopped', detail: null } }));
      await waitFor(() => expect(document.querySelector('.keyboard-subtitle__capture-status')).toBeNull());
      expect(node.textContent).toBe(current.text);
    });
  }

  test('an empty recording cannot claim listening before native capture is ready', async () => {
    current = snapshot({ text: '', originalText: '', phase: 'recording', audioState: { state: 'starting', detail: null } });
    renderBoard(); await waitFor(() => expect(document.querySelector('.keyboard-subtitle__capture-status[data-state="preparing"]')).toBeTruthy());
    expect(screen.queryByText('Recording…')).toBeNull();
    expect(screen.queryByText('Listening…')).toBeNull();
    act(() => publish({ audioState: { state: 'capturing', detail: null } }));
    await screen.findByText('Listening…');
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('');
    act(() => publish({ phase: 'finalizing' }));
    await waitFor(() => expect(document.querySelector('.keyboard-subtitle__capture-status[data-state="finishing"]')).toBeTruthy());
    expect(document.querySelector('.keyboard-subtitle__listen-bars')).toBeNull();
  });

  test('native editor capture preparation is visible while the save guard remains idle and stopped', async () => {
    current = snapshot({ capturePreparing: true, editSessionId: 'native-editor-lease', phase: 'idle',
      audioState: { state: 'stopped', detail: null } });
    renderBoard(); await waitFor(() => expect(document.querySelector('.keyboard-subtitle__capture-status[data-state="preparing"]')).toBeTruthy());
    expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe(current.text);
    expect(document.querySelector('.keyboard-subtitle__prepare-spinner')).toBeTruthy();
    expect(document.querySelector('.keyboard-subtitle__listen-bars')).toBeNull();
    expect(current.phase).toBe('idle'); expect(current.audioState.state).toBe('stopped');
    expect(actions).toHaveLength(0);
    expect(screen.getByRole('button', { name: 'Cancel' }).hasAttribute('disabled')).toBe(false);
    const stop = screen.getByRole('button', { name: 'Stop recording' });
    expect(stop.hasAttribute('disabled')).toBe(false);
    fireEvent.click(stop);
    await waitFor(() => expect(actions).toEqual([{ action: 'stop', documentId: 'document-1', revision: 3, cancelGeneration: 0 }]));
  });

  test('partial speech continues without inventing a newline and preserves supplier/manual line breaks', async () => {
    current = snapshot({ text: '手动第一行\n已有正文', originalText: '手动第一行\n已有正文', phase: 'recording',
      audioState: { state: 'capturing', detail: null }, partial: '继续🙂', dictationSelection: null });
    renderBoard();
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Final text' }).textContent).toBe('手动第一行\n已有正文继续🙂'));
    const node = screen.getByRole('textbox', { name: 'Final text' });
    expect(subtitleTextMap(node).text).toBe('手动第一行\n已有正文');
    act(() => publish({ partial: '\n供应商换行' }));
    expect(node.textContent).toBe('手动第一行\n已有正文\n供应商换行');
    act(() => publish({ text: '手动第一行\n已有正文\n', partial: '保留手动尾换行' }));
    expect(node.textContent).toBe('手动第一行\n已有正文\n保留手动尾换行');
  });

  test('native font settings apply to the single editable body without changing its text or selection', async () => {
    editBackend(); renderBoard(); const node = await focusEditor();
    expect(node.parentElement!.style.fontSize).toBe('14px');
    select(node, 3);
    const textNode = window.getSelection()!.anchorNode;
    act(() => publish({ settings: { ...current.settings, subtitleFontSize: 24 } }));
    expect(node.parentElement!.style.fontSize).toBe('24px');
    expect(node.textContent).toBe(current.text);
    expect(window.getSelection()!.anchorNode).toBe(textNode);
    expect(window.getSelection()!.anchorOffset).toBe(3);
  });

  test('changing body font size remeasures the read-only caret layout without resizing the recording label', async () => {
    await beginDictation(1);
    await waitFor(() => expect(document.querySelector('.keyboard-subtitle__caret')).toBeTruthy());
    Object.defineProperty(Range.prototype, 'getClientRects', { configurable: true, value: () => [new DOMRect(80, 32, 0, 20)] });
    act(() => publish({ settings: { ...current.settings, subtitleFontSize: 18 } }));
    expect(document.querySelector<HTMLElement>('.keyboard-subtitle__caret')!.style.left).toBe('80px');
    expect(document.querySelector<HTMLElement>('.keyboard-subtitle__caret')!.style.height).toBe('20px');
    expect(document.querySelector<HTMLElement>('.keyboard-subtitle__text')!.style.fontSize).toBe('18px');
    expect(document.querySelector<HTMLElement>('.keyboard-subtitle__capture-status')!.style.fontSize).toBe('');
  });

});
