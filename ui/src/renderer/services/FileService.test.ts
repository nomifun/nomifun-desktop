/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../test/setup-dom.ts';
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test';
import { act, cleanup, renderHook } from '@testing-library/react';
import * as ipcBridge from '@/common/adapter/ipcBridge';
import type { ConversationId } from '@/common/types/ids';
import * as uploadState from '@/renderer/hooks/file/useUploadState';
import { FileService } from './FileService';

class FakeUploadXhr extends EventTarget {
  static instances: FakeUploadXhr[] = [];
  upload = new EventTarget();
  status = 200;
  statusText = 'OK';
  responseText = '';
  method = '';
  url = '';
  headers = new Map<string, string>();
  body?: FormData;
  aborted = false;

  constructor() {
    super();
    FakeUploadXhr.instances.push(this);
  }

  open(method: string, url: string) {
    this.method = method;
    this.url = url;
  }

  setRequestHeader(name: string, value: string) {
    this.headers.set(name, value);
  }

  send(body: FormData) {
    this.body = body;
  }

  abort() {
    this.aborted = true;
    this.dispatchEvent(new Event('abort'));
  }

  progress(loaded: number, total: number) {
    this.upload.dispatchEvent(new ProgressEvent('progress', { lengthComputable: true, loaded, total }));
  }

  complete(path: string) {
    this.responseText = JSON.stringify({ success: true, data: path });
    this.dispatchEvent(new Event('load'));
  }
}

const originalXhr = globalThis.XMLHttpRequest;
const conversationId = '01900000-0000-7000-8000-000000000021' as ConversationId;
let restore: Array<() => void> = [];

function watch<T extends { mockRestore(): void }>(spy: T): T {
  restore.push(() => spy.mockRestore());
  return spy;
}

function fileList(...files: File[]): FileList {
  return Object.assign(files, { item: (index: number) => files[index] ?? null }) as unknown as FileList;
}

beforeEach(() => {
  FakeUploadXhr.instances = [];
  globalThis.XMLHttpRequest = FakeUploadXhr as unknown as typeof XMLHttpRequest;
  restore = [];
});

afterEach(async () => {
  uploadState.abortUploads({ source: 'sendbox' });
  await Promise.resolve();
  cleanup();
  restore.reverse().forEach((run) => run());
  globalThis.XMLHttpRequest = originalXhr;
});

describe('FileService native dropped paths', () => {
  test('keeps native Unicode and Windows paths in drop order without starting uploads', async () => {
    const paths = ['C:\\用户\\附件 空格\\截图.png', 'D:\\资料\\计划.pdf'] as const;
    const metadata = watch(spyOn(ipcBridge.fs.getFileMetadata, 'invoke')).mockImplementation(async ({ path }) => ({
      name: path === paths[0] ? '截图.png' : '计划.pdf',
      // Attachment identity follows the selected native path, even if metadata resolves an alias.
      path: '/resolved/alias',
      size: path === paths[0] ? 12 : 34,
      type: path === paths[0] ? 'png' : 'pdf',
      lastModified: 1234,
      isDirectory: false,
    }));
    const tracker = watch(spyOn(uploadState, 'trackUpload'));
    const fetch = watch(spyOn(globalThis, 'fetch')).mockRejectedValue(new Error('Unexpected HTTP request'));

    expect(await FileService.processNativePaths(paths)).toEqual([
      { name: '截图.png', path: paths[0], size: 12, type: 'png', lastModified: 1234 },
      { name: '计划.pdf', path: paths[1], size: 34, type: 'pdf', lastModified: 1234 },
    ]);
    expect(metadata.mock.calls).toEqual([
      [{ path: paths[0], workspace: 'C:\\用户\\附件 空格\\' }],
      [{ path: paths[1], workspace: 'D:\\资料\\' }],
    ]);
    expect(tracker).not.toHaveBeenCalled();
    expect(fetch).not.toHaveBeenCalled();
    expect(FakeUploadXhr.instances).toHaveLength(0);
  });

  test.each([
    ['D:\\报告.pdf', 'D:\\'],
    ['\\\\server\\share\\报告.pdf', '\\\\server\\share\\'],
    ['/Volumes/资料/报告.pdf', '/Volumes/资料/'],
    ['/报告.pdf', '/'],
  ])('scopes metadata to the selected file parent: %s', async (path, workspace) => {
    const metadata = watch(spyOn(ipcBridge.fs.getFileMetadata, 'invoke')).mockImplementation(async request => {
      if (request.workspace !== workspace) throw new Error('Outside default roots');
      return { name: '报告.pdf', path: request.path, size: 1, type: 'pdf', lastModified: 0, isDirectory: false };
    });
    expect((await FileService.processNativePaths([path!]))[0]?.path).toBe(path);
    expect(metadata).toHaveBeenCalledWith({ path, workspace });
  });

  test('deduplicates a batch and continues past directories and unreadable files', async () => {
    const validPath = 'C:\\附件\\有效.txt';
    const metadata = watch(spyOn(ipcBridge.fs.getFileMetadata, 'invoke')).mockImplementation(async ({ path }) => {
      if (path === 'C:\\附件\\已删除.txt') throw new Error('File no longer exists');
      return {
        name: path === validPath ? '有效.txt' : '目录',
        path,
        size: 7,
        type: 'txt',
        lastModified: 42,
        isDirectory: path !== validPath,
      };
    });
    watch(spyOn(console, 'error')).mockImplementation(() => {});
    watch(spyOn(console, 'warn')).mockImplementation(() => {});
    const tracker = watch(spyOn(uploadState, 'trackUpload'));

    expect(await FileService.processNativePaths([
      'C:\\附件\\目录',
      'C:\\附件\\已删除.txt',
      validPath,
      validPath,
    ])).toEqual([{ name: '有效.txt', path: validPath, size: 7, type: 'txt', lastModified: 42 }]);
    expect(metadata.mock.calls.map(([input]) => input.path)).toEqual([
      'C:\\附件\\目录', 'C:\\附件\\已删除.txt', validPath,
    ]);
    expect(tracker).not.toHaveBeenCalled();
    expect(FakeUploadXhr.instances).toHaveLength(0);
  });
});

describe('FileService browser dropped files', () => {
  test('preserves files that already carry a native path without HTTP upload', async () => {
    const file = Object.assign(new File(['image'], '截图.png', { type: 'image/png', lastModified: 123 }), {
      path: 'C:\\用户\\截图.png',
    });
    const tracker = watch(spyOn(uploadState, 'trackUpload'));

    expect(await FileService.processDroppedFiles(fileList(file), conversationId)).toEqual([{
      name: file.name, path: file.path, size: file.size, type: file.type, lastModified: file.lastModified,
    }]);
    expect(tracker).not.toHaveBeenCalled();
    expect(FakeUploadXhr.instances).toHaveLength(0);
  });

  test('WebUI uploads retain conversation binding and progress, then release the tracker', async () => {
    const file = new File(['document'], '资料.txt', { type: 'text/plain', lastModified: 456 });
    const hook = renderHook(() => uploadState.useActiveUploads('sendbox'));
    let pending!: ReturnType<typeof FileService.processDroppedFiles>;
    act(() => {
      pending = FileService.processDroppedFiles(fileList(file), conversationId);
    });
    const xhr = FakeUploadXhr.instances[0];

    expect(xhr.method).toBe('POST');
    expect(xhr.url).toEndWith('/api/fs/upload');
    expect(xhr.body?.get('conversation_id')).toBe(conversationId);
    expect((xhr.body?.get('file') as File).name).toBe(file.name);
    expect(hook.result.current).toEqual([expect.objectContaining({
      name: file.name, size: file.size, percent: 0, source: 'sendbox', conversationId,
    })]);

    act(() => xhr.progress(3, 8));
    expect(hook.result.current[0].percent).toBe(38);

    let result: Awaited<typeof pending> = [];
    await act(async () => {
      xhr.complete('/workspace/uploads/资料.txt');
      result = await pending;
    });
    expect(result).toEqual([{
      name: file.name, path: '/workspace/uploads/资料.txt', size: file.size, type: file.type, lastModified: 456,
    }]);
    expect(hook.result.current).toHaveLength(0);
  });

  test('cancelling a WebUI upload aborts its XHR and continues to other dropped files', async () => {
    const hook = renderHook(() => uploadState.useActiveUploads('sendbox'));
    const nativeFile = Object.assign(new File(['keep me'], 'keep.txt', { lastModified: 789 }), {
      path: 'C:\\附件\\keep.txt',
    });
    let pending!: ReturnType<typeof FileService.processDroppedFiles>;
    act(() => {
      pending = FileService.processDroppedFiles(fileList(new File(['cancel me'], 'cancel.txt'), nativeFile), conversationId);
    });
    const xhr = FakeUploadXhr.instances[0];
    let result: Awaited<typeof pending> = [];

    await act(async () => {
      uploadState.abortUpload(hook.result.current[0].id);
      result = await pending;
    });

    expect(xhr.aborted).toBe(true);
    expect(result).toEqual([{
      name: nativeFile.name, path: nativeFile.path, size: nativeFile.size,
      type: nativeFile.type, lastModified: nativeFile.lastModified,
    }]);
    expect(FakeUploadXhr.instances).toHaveLength(1);
    expect(hook.result.current).toHaveLength(0);
  });
});
