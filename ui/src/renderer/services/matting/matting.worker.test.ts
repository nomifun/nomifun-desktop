/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import * as inference from './onnxMatte';
import { handleMatteRequest } from './matting.worker';
import type { MatteRequest, MatteResponse } from './mattingProtocol';

const canvasDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'OffscreenCanvas');

/** Opaque gray scene with a red subject; quick cutout can recover it after AI fails. */
function makeBitmap(): ImageBitmap {
  const data = new Uint8ClampedArray(12 * 12 * 4);
  for (let y = 0; y < 12; y++) {
    for (let x = 0; x < 12; x++) {
      const subject = x >= 3 && x <= 8 && y >= 3 && y <= 8;
      data.set(subject ? [200, 40, 40, 255] : [220, 220, 220, 255], (y * 12 + x) * 4);
    }
  }
  return { width: 12, height: 12, data, close: mock(() => {}) } as unknown as ImageBitmap;
}

class TestCanvas {
  private data: Uint8ClampedArray<ArrayBuffer>;
  constructor(public width: number, public height: number) {
    this.data = new Uint8ClampedArray(width * height * 4);
  }
  getContext() {
    return {
      drawImage: (source: { data?: Uint8ClampedArray<ArrayBuffer> }) => {
        if (source.data) this.data = source.data.slice();
      },
      getImageData: () => ({ data: this.data, width: this.width, height: this.height }),
      putImageData: (image: ImageData) => { this.data = image.data; },
      imageSmoothingEnabled: true,
      imageSmoothingQuality: 'high',
    };
  }
  async convertToBlob(): Promise<Blob> { return new Blob(['cutout'], { type: 'image/webp' }); }
}

beforeEach(() => {
  Object.defineProperty(globalThis, 'OffscreenCanvas', { configurable: true, value: TestCanvas });
});

afterEach(() => {
  mock.restore();
  if (canvasDescriptor) Object.defineProperty(globalThis, 'OffscreenCanvas', canvasDescriptor);
  else Reflect.deleteProperty(globalThis, 'OffscreenCanvas');
});

async function run(overrides: Partial<MatteRequest> = {}): Promise<MatteResponse[]> {
  const messages: MatteResponse[] = [];
  await handleMatteRequest({ id: 7, type: 'matte', bitmap: makeBitmap(), mode: 'auto', ...overrides }, message => messages.push(message));
  return messages;
}

test('hands the explicit model Blob to inference and reports successful AI cutout', async () => {
  const model = new Blob([new Uint8Array([1, 2, 3])]);
  const alpha = new Uint8ClampedArray(12 * 12);
  alpha.fill(255, 40, 70);
  const infer = spyOn(inference, 'mlMatte').mockImplementation(async (_image, receivedModel, progress) => {
    expect(receivedModel).toBe(model);
    expect([...new Uint8Array(await receivedModel.arrayBuffer())]).toEqual([1, 2, 3]);
    progress?.('infer');
    return alpha;
  });
  const messages = await run({ model });
  expect(infer).toHaveBeenCalledTimes(1);
  expect(messages.some(message => message.type === 'fallback')).toBe(false);
  expect(messages).toContainEqual({ id: 7, type: 'progress', phase: 'infer' });
  expect(messages.at(-1)).toMatchObject({ id: 7, type: 'done', method: 'ml' });
});

test.each([undefined, new Blob()])('an absent or empty model falls back without touching inference', async model => {
  const infer = spyOn(inference, 'mlMatte').mockRejectedValue(new Error('must not initialize'));
  const messages = await run({ model });
  expect(infer).not.toHaveBeenCalled();
  expect(messages[0]).toEqual({ id: 7, type: 'fallback', reason: 'model_unavailable' });
  expect(messages.at(-1)).toMatchObject({ type: 'done', method: 'heuristic' });
});

test('an inference exception is classified separately from model acquisition failure', async () => {
  spyOn(inference, 'mlMatte').mockRejectedValue(new Error('WASM initialization failed'));
  const messages = await run({ model: new Blob(['model']) });
  expect(messages[0]).toEqual({ id: 7, type: 'fallback', reason: 'inference_failed', detail: 'WASM initialization failed' });
  expect(messages.at(-1)).toMatchObject({ type: 'done', method: 'heuristic' });
});

test('a valid inference with an unsuitable matte is classified as an image issue', async () => {
  spyOn(inference, 'mlMatte').mockResolvedValue(new Uint8ClampedArray(12 * 12).fill(255));
  const messages = await run({ model: new Blob(['model']) });
  expect(messages[0]).toEqual({ id: 7, type: 'fallback', reason: 'unsuitable_image' });
  expect(messages.at(-1)).toMatchObject({ type: 'done', method: 'heuristic' });
});

test('manual quick cutout never initializes AI or emits a misleading AI failure', async () => {
  const infer = spyOn(inference, 'mlMatte').mockRejectedValue(new Error('must not initialize'));
  const messages = await run({ mode: 'heuristic' });
  expect(infer).not.toHaveBeenCalled();
  expect(messages.some(message => message.type === 'fallback')).toBe(false);
  expect(messages.at(-1)).toMatchObject({ type: 'done', method: 'heuristic' });
});
