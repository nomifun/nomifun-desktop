/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../test/setup-dom.ts';
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test';
import type { ConversationId } from '@/common/types/ids';
import type { FileMetadata } from './FileService';
import { PasteService } from './PasteService';

class ClipboardUploadXhr extends EventTarget {
  static requests: FormData[] = [];
  upload = new EventTarget();
  status = 200;
  statusText = 'OK';
  responseText = '';

  open() {}
  setRequestHeader() {}

  send(body: FormData) {
    ClipboardUploadXhr.requests.push(body);
    const file = body.get('file') as File;
    this.responseText = JSON.stringify({ success: true, data: `/workspace/uploads/${file.name}` });
    this.dispatchEvent(new Event('load'));
  }
}

const originalXhr = globalThis.XMLHttpRequest;
const conversationId = '01900000-0000-7000-8000-000000000021' as ConversationId;

beforeEach(() => {
  ClipboardUploadXhr.requests = [];
  globalThis.XMLHttpRequest = ClipboardUploadXhr as unknown as typeof XMLHttpRequest;
});

afterEach(() => {
  globalThis.XMLHttpRequest = originalXhr;
});

function clipboard(...files: File[]): ClipboardEvent {
  const event = new Event('paste') as ClipboardEvent;
  Object.defineProperty(event, 'clipboardData', {
    value: { files, getData: () => 'clipboard filenames' },
  });
  return event;
}

test('clipboard images upload their bytes and receive a composer name even with an unofficial path', async () => {
  const file = Object.assign(new File(['image bytes'], 'image.png', { type: 'image/png' }), {
    path: '/untrusted/image.png',
  });
  const attachments: FileMetadata[] = [];
  expect(await PasteService.handlePaste(
    clipboard(file), ['.png'], files => attachments.push(...files), undefined,
    conversationId, 'sendbox', { next: () => 1 }
  )).toBe(true);

  expect(ClipboardUploadXhr.requests).toHaveLength(1);
  const request = ClipboardUploadXhr.requests[0];
  expect(request.get('conversation_id')).toBe(conversationId);
  expect(await (request.get('file') as File).text()).toBe('image bytes');
  expect(attachments).toEqual([expect.objectContaining({
    name: 'image-1.png', path: '/workspace/uploads/image-1.png', type: 'image/png', size: file.size,
  })]);
});

test('clipboard documents upload their bytes instead of accepting an unofficial host path', async () => {
  const file = Object.assign(new File(['document bytes'], '报告.txt', { type: 'text/plain' }), {
    path: '/untrusted/报告.txt',
  });
  const attachments: FileMetadata[] = [];
  expect(await PasteService.handlePaste(clipboard(file), ['.txt'], files => attachments.push(...files))).toBe(true);

  expect(ClipboardUploadXhr.requests).toHaveLength(1);
  expect(await (ClipboardUploadXhr.requests[0].get('file') as File).text()).toBe('document bytes');
  expect(attachments).toEqual([expect.objectContaining({
    name: '报告.txt', path: '/workspace/uploads/报告.txt', type: 'text/plain', size: file.size,
  })]);
});

test('clipboard extension filters still reject unsupported images and documents before upload', async () => {
  const warn = spyOn(console, 'warn').mockImplementation(() => {});
  try {
    const onFilesAdded = spyOn({ add: (_files: FileMetadata[]) => {} }, 'add');
    expect(await PasteService.handlePaste(clipboard(
      new File(['image'], 'image.avif', { type: 'image/avif' }),
      new File(['document'], 'report.pdf', { type: 'application/pdf' }),
    ), ['.png'], onFilesAdded)).toBe(true);

    expect(ClipboardUploadXhr.requests).toHaveLength(0);
    expect(onFilesAdded).not.toHaveBeenCalled();
  } finally {
    warn.mockRestore();
  }
});
