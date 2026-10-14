import '../../../../test/setup-dom.ts';
import { act, cleanup, createEvent, fireEvent, render, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { useRef } from 'react';
import { createPortal } from 'react-dom';
import { Message } from '@arco-design/web-react';
import * as runtime from '@/common/adapter/tauriRuntime';
import type { ConversationId } from '@/common/types/ids';
import * as webviewApi from '@tauri-apps/api/webview';
import { PhysicalPosition } from '@tauri-apps/api/dpi';
import type { EventCallback } from '@tauri-apps/api/event';
import { FileService, type FileMetadata } from '@/renderer/services/FileService';
import { useDragUpload } from './useDragUpload';
import { abortUploads, useUploadState } from './useUploadState';

const conversationId = '01900000-0000-7000-8000-000000000011' as ConversationId;
const otherConversationId = '01900000-0000-7000-8000-000000000012' as ConversationId;
const attachment: FileMetadata = { name: '参考.png', path: 'C:/资料/参考.png', size: 1, type: 'image/png', lastModified: 1 };
const file = new File(['x'], attachment.name, { type: attachment.type });
let restore: Array<() => void> = [];
let browser: ReturnType<typeof spyOn<typeof FileService, 'processDroppedFiles'>>;
let native: ReturnType<typeof spyOn<typeof FileService, 'processNativePaths'>>;
let runtimeSpy: ReturnType<typeof spyOn<typeof runtime, 'isTauriRuntime'>>;
let warning: ReturnType<typeof spyOn<typeof Message, 'warning'>>;
let error: ReturnType<typeof spyOn<typeof Message, 'error'>>;

function watch<T extends { mockRestore(): void }>(spy: T): T {
  restore.push(() => spy.mockRestore());
  return spy;
}

beforeEach(() => {
  runtimeSpy = watch(spyOn(runtime, 'isTauriRuntime').mockReturnValue(false));
  browser = watch(spyOn(FileService, 'processDroppedFiles').mockResolvedValue([attachment]));
  native = watch(spyOn(FileService, 'processNativePaths').mockResolvedValue([attachment]));
  warning = watch(spyOn(Message, 'warning').mockImplementation(() => undefined as never));
  error = watch(spyOn(Message, 'error').mockImplementation(() => undefined as never));
});
afterEach(() => { cleanup(); restore.splice(0).reverse().forEach(dispose => dispose()); });

function fixture(onFilesAdded = mock((_files: FileMetadata[]) => {})) {
  function Host() {
    const targetRef = useRef<HTMLDivElement>(null);
    const drop = useDragUpload({ targetRef, onFilesAdded, conversation_id: conversationId });
    const { isUploading } = useUploadState('sendbox');
    return <div ref={targetRef} {...drop.dragHandlers} data-testid='surface' data-dragging={drop.isFileDragging} data-uploading={isUploading}>
      <textarea defaultValue='保留草稿' /><button type='button'>添加</button>
    </div>;
  }
  const view = render(<Host />);
  return { ...view, onFilesAdded, surface: view.getByTestId('surface'), input: view.getByRole('textbox') as HTMLTextAreaElement };
}

const transfer = () => ({ types: ['Files'], files: [file], dropEffect: 'none' });
async function flush() { await act(async () => { await Promise.resolve(); }); }

test('browser drops append through the existing attachment callback without changing the draft', async () => {
  const view = fixture();
  const dataTransfer = transfer();
  fireEvent.dragEnter(view.surface, { dataTransfer });
  fireEvent.dragEnter(view.input, { dataTransfer });
  const over = createEvent.dragOver(view.input, { dataTransfer });
  fireEvent(view.input, over);
  expect((over as DragEvent).dataTransfer?.dropEffect).toBe('copy');
  expect(view.surface.dataset.dragging).toBe('true');
  fireEvent.dragLeave(view.input, { dataTransfer });
  expect(view.surface.dataset.dragging).toBe('true');
  expect(fireEvent.drop(view.input, { dataTransfer })).toBe(false);
  await flush();
  expect(view.surface.dataset.dragging).toBe('false');
  expect(Array.from(browser.mock.calls[0]![0])).toEqual([file]);
  expect(browser.mock.calls[0]!.slice(1)).toEqual([conversationId, 'sendbox']);
  expect(view.onFilesAdded).toHaveBeenCalledTimes(1);
  expect(view.onFilesAdded).toHaveBeenCalledWith([attachment]);
  expect(view.input.value).toBe('保留草稿');
  expect(native).not.toHaveBeenCalled();
});

test('dragover does not inflate enter/leave counts or leave a stuck highlight', () => {
  const view = fixture();
  const dataTransfer = transfer();
  fireEvent.dragEnter(view.surface, { dataTransfer });
  fireEvent.dragOver(view.input, { dataTransfer });
  fireEvent.dragOver(view.input, { dataTransfer });
  fireEvent.dragLeave(view.surface, { dataTransfer });
  expect(view.surface.dataset.dragging).toBe('false');
});

test('text and URL drags keep their normal editor behavior', () => {
  const view = fixture();
  const dataTransfer = { types: ['text/plain', 'text/uri-list'], files: [], items: [] };
  expect(fireEvent.dragEnter(view.input, { dataTransfer })).toBe(true);
  expect(fireEvent.dragOver(view.input, { dataTransfer })).toBe(true);
  expect(fireEvent.drop(view.input, { dataTransfer })).toBe(true);
  expect(view.surface.dataset.dragging).toBe('false');
  expect(browser).not.toHaveBeenCalled();
  expect(view.onFilesAdded).not.toHaveBeenCalled();
});

test('a surface without an attachment callback does not intercept files', () => {
  function Host() {
    const targetRef = useRef<HTMLDivElement>(null);
    const drop = useDragUpload({ targetRef });
    return <div ref={targetRef} {...drop.dragHandlers} data-dragging={drop.isFileDragging} data-testid='surface' />;
  }
  const view = render(<Host />);
  const surface = view.getByTestId('surface');
  expect(fireEvent.dragEnter(surface, { dataTransfer: transfer() })).toBe(true);
  expect(fireEvent.drop(surface, { dataTransfer: transfer() })).toBe(true);
  expect(surface.dataset.dragging).toBe('false');
  expect(browser).not.toHaveBeenCalled();
});

test('upload-only surfaces retain their browser staging flow in the desktop shell', async () => {
  runtimeSpy.mockReturnValue(true);
  const onFilesAdded = mock(() => {});
  function Host() {
    const drop = useDragUpload({ source: 'requirement', onFilesAdded });
    return <div {...drop.dragHandlers} data-testid='surface' />;
  }
  const view = render(<Host />);
  fireEvent.drop(view.getByTestId('surface'), { dataTransfer: transfer() });
  await flush();
  expect(browser.mock.calls[0]?.slice(1)).toEqual([undefined, 'requirement']);
  expect(native).not.toHaveBeenCalled();
  expect(onFilesAdded).toHaveBeenCalledWith([attachment]);
});

test('browser directories are excluded while ordinary files still upload', async () => {
  const view = fixture();
  const folder = new File([], 'folder');
  fireEvent.drop(view.input, { dataTransfer: { types: ['Files'], files: [folder, file], items: [
    { kind: 'string' },
    { kind: 'file', webkitGetAsEntry: () => ({ isDirectory: true }) },
    { kind: 'file', webkitGetAsEntry: () => ({ isDirectory: false }) },
  ] } });
  await flush();
  expect(Array.from(browser.mock.calls[0]![0])).toEqual([file]);
  expect(view.onFilesAdded).toHaveBeenCalledWith([attachment]);
});

test.each(['conversation change', 'unmount'] as const)('a pending drop cannot attach after %s', async mode => {
  let complete!: (files: FileMetadata[]) => void;
  browser.mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  const onFilesAdded = mock(() => {});
  const targetRef = { current: null };
  const view = renderHook(({ id }) => useDragUpload({ targetRef, onFilesAdded, conversation_id: id }), {
    initialProps: { id: conversationId },
  });
  act(() => view.result.current.dragHandlers.onDrop({ target: document.body, currentTarget: document.body,
    dataTransfer: transfer(), preventDefault() {}, stopPropagation() {} } as unknown as React.DragEvent));
  if (mode === 'unmount') view.unmount();
  else view.rerender({ id: otherConversationId });
  await act(async () => { complete([attachment]); });
  expect(onFilesAdded).not.toHaveBeenCalled();
});

test('file drags over portaled controls outside the composer do not add attachments', async () => {
  const onFilesAdded = mock(() => {});
  function Host() {
    const targetRef = useRef<HTMLDivElement>(null);
    const drop = useDragUpload({ targetRef, onFilesAdded });
    return <div ref={targetRef} {...drop.dragHandlers}>
      {createPortal(<button type='button'>弹出菜单</button>, document.body)}
    </div>;
  }
  const view = render(<Host />);
  fireEvent.drop(view.getByRole('button'), { dataTransfer: transfer() });
  await flush();
  expect(browser).not.toHaveBeenCalled();
  expect(onFilesAdded).not.toHaveBeenCalled();
});

test.each([false, true])('native inspection holds the send guard and respects cancellation: %s', async cancel => {
  runtimeSpy.mockReturnValue(true);
  let complete!: (files: FileMetadata[]) => void;
  native.mockImplementation(() => new Promise(resolve => { complete = resolve; }));
  let emit!: EventCallback<webviewApi.DragDropEvent>;
  watch(spyOn(webviewApi, 'getCurrentWebview').mockReturnValue({ onDragDropEvent: (handler: typeof emit) => {
    emit = handler;
    return Promise.resolve(() => {});
  } } as unknown as webviewApi.Webview));
  const view = fixture();
  watch(spyOn(view.surface, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 500, 100)));
  watch(spyOn(document, 'elementFromPoint').mockReturnValue(view.input));
  await flush();
  act(() => emit({ event: 'tauri://drag-drop', id: 1, payload: {
    type: 'drop', paths: [attachment.path], position: new PhysicalPosition(20, 20),
  } }));
  expect(view.surface.dataset.uploading).toBe('true');
  expect(view.onFilesAdded).not.toHaveBeenCalled();
  if (cancel) act(() => { abortUploads({ source: 'sendbox' }); });
  await act(async () => { complete([attachment]); });
  expect(view.surface.dataset.uploading).toBe('false');
  if (cancel) expect(view.onFilesAdded).not.toHaveBeenCalled();
  else expect(view.onFilesAdded).toHaveBeenCalledWith([attachment]);
  expect(warning).not.toHaveBeenCalled();
  expect(error).not.toHaveBeenCalled();
});

test('native drops use host paths once even if a DOM file drop also fires', async () => {
  runtimeSpy.mockReturnValue(true);
  let emit!: EventCallback<webviewApi.DragDropEvent>;
  watch(spyOn(webviewApi, 'getCurrentWebview').mockReturnValue({ onDragDropEvent: (handler: typeof emit) => {
    emit = handler;
    return Promise.resolve(() => {});
  } } as unknown as webviewApi.Webview));
  const view = fixture();
  watch(spyOn(view.surface, 'getBoundingClientRect').mockReturnValue(new DOMRect(0, 0, 500, 100)));
  watch(spyOn(document, 'elementFromPoint').mockReturnValue(view.input));
  await flush();
  const position = new PhysicalPosition(20, 20);
  act(() => emit({ event: 'tauri://drag-enter', id: 1, payload: { type: 'enter', paths: [attachment.path], position } }));
  expect(view.surface.dataset.dragging).toBe('true');
  act(() => emit({ event: 'tauri://drag-drop', id: 1, payload: { type: 'drop', paths: [attachment.path], position } }));
  fireEvent.drop(view.input, { dataTransfer: transfer() });
  await flush();
  expect(native).toHaveBeenCalledWith([attachment.path]);
  expect(browser).not.toHaveBeenCalled();
  expect(view.onFilesAdded).toHaveBeenCalledTimes(1);
  expect(view.onFilesAdded).toHaveBeenCalledWith([attachment]);
  expect(view.input.value).toBe('保留草稿');
  expect(view.surface.dataset.dragging).toBe('false');
});

test('failed drops report feedback while cancelled uploads remain quiet', async () => {
  const view = fixture();
  browser.mockRejectedValueOnce(new Error('FILE_TOO_LARGE')).mockResolvedValueOnce([]);
  watch(spyOn(console, 'error').mockImplementation(() => {}));
  fireEvent.drop(view.input, { dataTransfer: transfer() });
  await flush();
  expect(error).toHaveBeenCalledTimes(1);
  fireEvent.drop(view.input, { dataTransfer: transfer() });
  await flush();
  expect(warning).not.toHaveBeenCalled();
  expect(view.onFilesAdded).not.toHaveBeenCalled();
  expect(view.surface.dataset.dragging).toBe('false');
});

test('a browser folder drop reports why no attachment was added', async () => {
  const view = fixture();
  fireEvent.drop(view.input, { dataTransfer: { types: ['Files'], files: [new File([], 'folder')],
    items: [{ kind: 'file', webkitGetAsEntry: () => ({ isDirectory: true }) }],
  } });
  await flush();
  expect(warning).toHaveBeenCalledTimes(1);
  expect(browser).not.toHaveBeenCalled();
  expect(view.onFilesAdded).not.toHaveBeenCalled();
});
