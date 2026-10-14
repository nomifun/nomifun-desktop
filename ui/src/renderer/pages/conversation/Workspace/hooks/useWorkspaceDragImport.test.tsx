import '../../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import type { DragEvent, RefObject } from 'react';
import type { TFunction } from 'i18next';
import * as webviewApi from '@tauri-apps/api/webview';
import type { EventCallback } from '@tauri-apps/api/event';
import { PhysicalPosition } from '@tauri-apps/api/dpi';
import { ipcBridge } from '@/common';
import { parseConversationId } from '@/common/types/ids';
import * as runtime from '@/common/adapter/tauriRuntime';
import { FileService } from '@/renderer/services/FileService';
import type { MessageApi } from '../types';
import { useWorkspaceDragImport } from './useWorkspaceDragImport';

const sourceKey = parseConversationId('0190f5fe-7c00-7a00-8000-000000000991');
const t = ((key: string, options?: { defaultValue?: string }) => options?.defaultValue ?? key) as TFunction<'translation'>;
let runtimeSpy: ReturnType<typeof spyOn<typeof runtime, 'isTauriRuntime'>>;
let webviewSpy: ReturnType<typeof spyOn<typeof webviewApi, 'getCurrentWebview'>>;
let hitSpy: ReturnType<typeof spyOn<typeof document, 'elementFromPoint'>>;
let metadataSpy: ReturnType<typeof spyOn<typeof ipcBridge.fs.getFileMetadata, 'invoke'>>;
let uploadSpy: ReturnType<typeof spyOn<typeof FileService, 'processDroppedFiles'>>;
let handler: EventCallback<webviewApi.DragDropEvent> | undefined;
let target: HTMLDivElement;
let targetRef: RefObject<HTMLElement | null>;
let originalDpr: PropertyDescriptor | undefined;

beforeEach(() => {
  originalDpr = Object.getOwnPropertyDescriptor(window, 'devicePixelRatio');
  Object.defineProperty(window, 'devicePixelRatio', { value: 1, configurable: true });
  target = document.createElement('div');
  document.body.append(target);
  target.getBoundingClientRect = () => new DOMRect(400, 20, 200, 400);
  targetRef = { current: target };
  handler = undefined;
  runtimeSpy = spyOn(runtime, 'isTauriRuntime').mockReturnValue(true);
  hitSpy = spyOn(document, 'elementFromPoint').mockReturnValue(target);
  webviewSpy = spyOn(webviewApi, 'getCurrentWebview').mockReturnValue({
    onDragDropEvent: (listener: EventCallback<webviewApi.DragDropEvent>) => {
      handler = listener;
      return Promise.resolve(() => {});
    },
  } as unknown as webviewApi.Webview);
  metadataSpy = spyOn(ipcBridge.fs.getFileMetadata, 'invoke').mockImplementation(async ({ path }) => ({
    path, name: path.split('/').pop()!, size: 12, type: '', lastModified: 1, isDirectory: path === 'C:/folder',
  }));
  uploadSpy = spyOn(FileService, 'processDroppedFiles').mockResolvedValue([
    { path: '/stored/report.txt', name: 'report.txt', size: 12, type: 'text/plain', lastModified: 1 },
  ]);
});

afterEach(() => {
  cleanup();
  target.remove();
  runtimeSpy.mockRestore();
  webviewSpy.mockRestore();
  hitSpy.mockRestore();
  metadataSpy.mockRestore();
  uploadSpy.mockRestore();
  if (originalDpr) Object.defineProperty(window, 'devicePixelRatio', originalDpr);
  else Reflect.deleteProperty(window, 'devicePixelRatio');
});

function fixture(enabled = true) {
  const onFilesDropped = mock((_files: Array<{ path: string; name: string }>) => {});
  const warning = mock(() => {});
  const error = mock(() => {});
  const messageApi = { warning, error } as unknown as MessageApi;
  const hook = renderHook(() => useWorkspaceDragImport({ targetRef, enabled, onFilesDropped, sourceKey, messageApi, t }));
  return { hook, onFilesDropped, warning, error };
}

async function emit(payload: webviewApi.DragDropEvent) {
  await act(async () => { handler!({ event: 'tauri://drag-drop', id: 1, payload }); });
}

function browserEvent(files: File[]): DragEvent {
  const dataTransfer = { files: Object.assign(files, { item: (index: number) => files[index] ?? null }), items: [] };
  return { dataTransfer, nativeEvent: { dataTransfer }, preventDefault: mock(() => {}), stopPropagation: mock(() => {}) } as unknown as DragEvent;
}

test('workspace only imports native drops inside its surface and preserves directory import', async () => {
  const f = fixture();
  await act(async () => { await Promise.resolve(); });
  await emit({ type: 'drop', paths: ['C:/report.txt'], position: new PhysicalPosition(100, 100) });
  expect(metadataSpy).not.toHaveBeenCalled();
  expect(f.onFilesDropped).not.toHaveBeenCalled();
  await emit({ type: 'enter', paths: ['C:/report.txt'], position: new PhysicalPosition(500, 100) });
  expect(f.hook.result.current.isDragging).toBe(true);
  await emit({ type: 'drop', paths: ['C:/report.txt', 'C:/report.txt', 'C:/folder'], position: new PhysicalPosition(500, 100) });
  expect(f.hook.result.current.isDragging).toBe(false);
  expect(metadataSpy).toHaveBeenCalledTimes(2);
  expect(f.onFilesDropped.mock.calls).toEqual([[[
    { path: 'C:/report.txt', name: 'report.txt' },
    { path: 'C:/folder', name: 'folder' },
  ]]]);
  expect(uploadSpy).not.toHaveBeenCalled();
  expect(f.warning).not.toHaveBeenCalled();
});

test('Tauri DOM events add no duplicate upload or empty-drop warning', async () => {
  const f = fixture();
  const event = browserEvent([new File(['report'], 'report.txt')]);
  act(() => { f.hook.result.current.dragHandlers.onDragEnter(event); });
  expect(f.hook.result.current.isDragging).toBe(false);
  await act(async () => { await f.hook.result.current.dragHandlers.onDrop(event); });
  expect(uploadSpy).not.toHaveBeenCalled();
  expect(f.onFilesDropped).not.toHaveBeenCalled();
  expect(f.warning).not.toHaveBeenCalled();
});

test('workspace without upload capability subscribes to no native drops and accepts no DOM upload', async () => {
  const f = fixture(false);
  await act(async () => { await Promise.resolve(); });
  expect(webviewSpy).not.toHaveBeenCalled();
  runtimeSpy.mockReturnValue(false);
  await act(async () => { await f.hook.result.current.dragHandlers.onDrop(browserEvent([new File(['a'], 'a.txt')])); });
  expect(uploadSpy).not.toHaveBeenCalled();
  expect(f.onFilesDropped).not.toHaveBeenCalled();
});

test('WebUI continues uploading browser files with workspace progress scoping', async () => {
  runtimeSpy.mockReturnValue(false);
  const f = fixture();
  const files = [new File(['report'], 'report.txt')];
  await act(async () => { await f.hook.result.current.dragHandlers.onDrop(browserEvent(files)); });
  expect(webviewSpy).not.toHaveBeenCalled();
  expect(uploadSpy).toHaveBeenCalledTimes(1);
  expect(uploadSpy.mock.calls[0]![1]).toBe(sourceKey);
  expect(uploadSpy.mock.calls[0]![2]).toBe('workspace');
  expect(f.onFilesDropped.mock.calls).toEqual([[[{ path: '/stored/report.txt', name: 'report.txt' }]]]);
});
