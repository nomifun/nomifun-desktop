/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import * as ort from 'onnxruntime-web/wasm';
import { mlMatte } from './onnxMatte';

afterEach(() => { mock.restore(); });

test('initializes from the supplied Blob, reuses its session, and releases a failed session before rebuilding', async () => {
  const bytes = new Uint8Array([1, 4, 9, 16]);
  const model = new Blob([bytes]);
  const readModel = spyOn(model, 'arrayBuffer');
  const output = () => new ort.Tensor('float32', new Float32Array([0, 0.5, 1, 0]), [1, 1, 2, 2]);
  let shouldFail = false;
  const release = mock(async () => {});
  const run = mock(async () => {
    if (shouldFail) throw new Error('session failed');
    return { output: output() };
  });
  const create = spyOn(ort.InferenceSession, 'create').mockImplementation(async (...args) => {
    expect([...new Uint8Array(args[0] as ArrayBuffer)]).toEqual([...bytes]);
    return { inputNames: ['input'], outputNames: ['output'], run, release } as unknown as ort.InferenceSession;
  });
  const image = { data: new Uint8ClampedArray(2 * 2 * 4).fill(255), width: 2, height: 2 };

  expect([...await mlMatte(image, model)]).toEqual([0, 128, 255, 0]);
  expect([...await mlMatte(image, model)]).toEqual([0, 128, 255, 0]);
  expect(create).toHaveBeenCalledTimes(1);
  expect(readModel).toHaveBeenCalledTimes(1);
  await expect(mlMatte(image, undefined as unknown as Blob)).rejects.toThrow('matting model Blob unavailable');
  await expect(mlMatte(image, new Blob())).rejects.toThrow('matting model Blob unavailable');
  expect(run).toHaveBeenCalledTimes(2);

  shouldFail = true;
  await expect(mlMatte(image, model)).rejects.toThrow('session failed');
  expect(release).toHaveBeenCalledTimes(1);
  shouldFail = false;
  await mlMatte(image, model);
  expect(create).toHaveBeenCalledTimes(2);
  expect(readModel).toHaveBeenCalledTimes(2);
});
