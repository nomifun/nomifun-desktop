/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';

import type { CreativeAsset, CreativeAssetPort } from '../../assets';
import { canvasCommands, canvasReducer, createInitialCanvasState } from '../core';
import { testDocument, testNode, testUuid } from '../core/testFixtures';
import {
  fillEmptyCanvasImageNodeFromAsset,
  uploadCanvasImageNodeAsset,
  uploadedCanvasImageNodeSize,
} from './imageNodeUpload';

const imageAsset = (overrides: Partial<CreativeAsset> = {}): CreativeAsset => ({
  id: testUuid(701),
  kind: 'image',
  title: '上传的图片.png',
  collection: null,
  tags: ['canvas-node-upload'],
  mimeType: 'image/png',
  width: 1920,
  height: 1080,
  bytes: 1024,
  inLibrary: true,
  textContent: null,
  origin: null,
  originalUrl: `/api/creative-studio/files/${testUuid(701)}`,
  thumbnailUrl: null,
  createdAt: 1,
  updatedAt: 1,
  ...overrides,
});

const thrownMessage = (action: () => void): string => {
  try {
    action();
    return '';
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
};

const rejectedMessage = async (action: () => Promise<unknown>): Promise<string> => {
  try {
    await action();
    return '';
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
};

describe('canvas image node upload projection', () => {
  test('uploads with one durable operation tag and recovers a lost response', async () => {
    const operationId = testUuid(704);
    const tag = `canvas-image-node-upload:${operationId}`;
    let metadata: Parameters<CreativeAssetPort['upload']>[1];
    const recovered = imageAsset({ tags: ['canvas-node-upload', tag] });
    const result = await uploadCanvasImageNodeAsset({
      port: {
        async upload(_file, nextMetadata) {
          metadata = nextMetadata;
          throw new Error('response lost');
        },
        async list(query) {
          expect(query?.tag).toBe(tag);
          return { items: [recovered], total: 1 };
        },
        async update() {
          return recovered;
        },
        async remove() {},
        url: () => recovered.originalUrl,
      },
      file: new File(['image'], 'upload.png', { type: 'image/png' }),
      operationId,
    });
    expect(metadata).toEqual({
      title: 'upload.png',
      tags: ['canvas-node-upload', tag],
      inLibrary: true,
    });
    expect(result).toEqual({
      asset: recovered,
      recoveredAfterResponseLoss: true,
    });
  });

  test('rejects unsupported and oversized files before calling the asset port', async () => {
    let uploads = 0;
    const port: CreativeAssetPort = {
      async upload() {
        uploads += 1;
        return imageAsset();
      },
      async list() {
        return { items: [], total: 0 };
      },
      async update() {
        return imageAsset();
      },
      async remove() {},
      url: () => imageAsset().originalUrl,
    };
    const unsupported = await rejectedMessage(() =>
      uploadCanvasImageNodeAsset({
        port,
        file: new File(['text'], 'note.txt', { type: 'text/plain' }),
        operationId: testUuid(705),
      })
    );
    const oversized = new File(['image'], 'large.png', { type: 'image/png' });
    Object.defineProperty(oversized, 'size', { value: 64 * 1024 * 1024 + 1 });
    const tooLarge = await rejectedMessage(() =>
      uploadCanvasImageNodeAsset({
        port,
        file: oversized,
        operationId: testUuid(706),
      })
    );
    expect(unsupported.includes('只接受真实图片文件')).toBe(true);
    expect(tooLarge.includes('不能超过 64 MB')).toBe(true);
    expect(uploads).toBe(0);
  });

  test('fills the exact empty node and preserves its graph identity', () => {
    const base = testNode('image', 1, {
      x: 120,
      y: 80,
      width: 340,
      height: 240,
      locked: true,
      groupId: testUuid(702),
    });
    const node = {
      ...base,
      data: {
        ...base.data,
        composer: {
          prompt: '保留这个草稿',
          model: { providerId: testUuid(710), model: 't2i-model' },
          interfaceMode: 'images' as const,
          quality: 'high' as const,
          width: 1024,
          height: 1024,
          aspectRatio: '1:1',
          count: 2,
        },
      },
    };
    const filled = fillEmptyCanvasImageNodeFromAsset(node, imageAsset());

    expect(filled).toMatchObject({
      id: node.id,
      position: node.position,
      groupId: node.groupId,
      zIndex: node.zIndex,
      locked: true,
      size: { width: 640, height: 360 },
      data: {
        assetId: testUuid(701),
        caption: '上传的图片.png',
        alt: '上传的图片.png',
        fit: 'contain',
        naturalSize: { width: 1920, height: 1080 },
        composer: {
          prompt: '保留这个草稿',
          model: null,
          interfaceMode: 'images',
          quality: 'high',
          width: 1024,
          height: 1024,
          aspectRatio: '1:1',
          count: 2,
        },
      },
    });
  });

  test('keeps the current display size when metadata has no real dimensions', () => {
    expect(
      uploadedCanvasImageNodeSize(imageAsset({ width: null, height: null }), {
        width: 340,
        height: 240,
      })
    ).toEqual({ width: 340, height: 240 });
  });

  test('updates, undoes, and redoes the same node identity', () => {
    const node = testNode('image', 1);
    let state = createInitialCanvasState({ document: testDocument([node]) });
    state = canvasReducer(
      state,
      canvasCommands.updateNode(fillEmptyCanvasImageNodeFromAsset(node, imageAsset()))
    );
    expect(state.document.nodes[0]).toMatchObject({
      id: node.id,
      data: { assetId: testUuid(701) },
    });
    state = canvasReducer(state, canvasCommands.undo());
    expect(state.document.nodes[0]).toMatchObject({
      id: node.id,
      data: { assetId: null },
    });
    state = canvasReducer(state, canvasCommands.redo());
    expect(state.document.nodes[0]).toMatchObject({
      id: node.id,
      data: { assetId: testUuid(701) },
    });
  });

  test('refuses to overwrite an existing image or accept another asset kind', () => {
    const empty = testNode('image', 1);
    const occupied = {
      ...empty,
      data: { ...empty.data, assetId: testUuid(703) },
    };
    expect(
      thrownMessage(() => fillEmptyCanvasImageNodeFromAsset(occupied, imageAsset())).includes(
        '未覆盖现有内容'
      )
    ).toBe(true);
    expect(
      thrownMessage(() =>
        fillEmptyCanvasImageNodeFromAsset(empty, imageAsset({ kind: 'video' }))
      ).includes('不是有效的真实图片素材')
    ).toBe(true);
  });
});
