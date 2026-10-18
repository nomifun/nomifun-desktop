/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 抠图 Worker：解码 →（透明直通）→ ML / 启发式泛洪 → 失效检测 → 应用 alpha →
 * 包围盒裁切 → 长边 ≤2048 重采样 → WebP 编码 → 回传 blob + aspect + headBox。
 *
 * 消费方按 Vite 惯例创建（本仓库 Worker 首例，以此为准）：
 *   new Worker(new URL('./matting.worker.ts', import.meta.url), { type: 'module' })
 *
 * 消息协议见 mattingProtocol.ts。主线程持有推理 watchdog：超时直接终止本 Worker，
 * 才能中断同步 WASM 并释放 session；Worker 内不做无法取消后台推理的 Promise 竞速。
 */

import { heuristicMatte, suggestHeadBox, type RawImage } from './heuristicMatte';
import { mlMatte } from './onnxMatte';
import type { MatteMethod, MatteRequest, MatteResponse } from './mattingProtocol';

/** 抠图失效错误码（UI 层译为"无法识别背景，请提供透明底图或更简单背景"）。 */
const MATTE_FAILED = 'MATTE_FAILED';

/** 已有透明像素（alpha<250）占比超过 1% → 视为透明底图，原样直通。 */
const TRANSPARENT_ALPHA = 250;
const TRANSPARENT_RATIO = 0.01;
/** 失效检测：前景（alpha>40）占比 >92% 或 <0.5% 视为抠图失效（审查 I1）。 */
const FG_ALPHA_THRESHOLD = 40;
const FG_MAX_RATIO = 0.92;
const FG_MIN_RATIO = 0.005;
/** 裁切：alpha>10 的包围盒外扩 8px。 */
const BBOX_ALPHA_THRESHOLD = 10;
const BBOX_MARGIN = 8;
/** 成品长边上限。 */
const MAX_OUTPUT_EDGE = 2048;

// 模块作用域 self 重声明：tsconfig lib 是 DOM（无 webworker），收窄到 Worker 实际能力。
declare const self: {
  postMessage(message: unknown): void;
  onmessage: ((ev: MessageEvent<MatteRequest>) => void) | null;
};

type Emit = (message: MatteResponse) => void;

// A document context may import the handler in tests without installing a window listener.
if (typeof document === 'undefined') {
  let pending = Promise.resolve();
  self.onmessage = (ev) => {
    const req = ev.data;
    if (!req || req.type !== 'matte') return;
    // Never run two sessions concurrently in one WASM runtime.
    pending = pending.then(() => handleMatteRequest(req, (message) => self.postMessage(message)));
  };
}

export async function handleMatteRequest(req: MatteRequest, emit: Emit): Promise<void> {
  const { id } = req;
  try {
    const result = await matte(req, emit);
    emit({ id, type: 'done', ...result });
  } catch (err) {
    emit({ id, type: 'error', message: err instanceof Error ? err.message : String(err) });
  }
}

function transparentRatio(data: Uint8ClampedArray): number {
  const n = data.length / 4;
  let count = 0;
  for (let i = 0; i < n; i++) {
    if (data[i * 4 + 3] < TRANSPARENT_ALPHA) count++;
  }
  return count / n;
}

function isMatteInvalid(alpha: Uint8ClampedArray): boolean {
  let fg = 0;
  for (let i = 0; i < alpha.length; i++) {
    if (alpha[i] > FG_ALPHA_THRESHOLD) fg++;
  }
  const ratio = fg / alpha.length;
  return ratio > FG_MAX_RATIO || ratio < FG_MIN_RATIO;
}

async function matte(req: MatteRequest, emit: Emit): Promise<{
  blob: Blob;
  aspect: number;
  headBox: { x: number; y: number; w: number };
  method: MatteMethod;
}> {
  const { id, bitmap, mode, tolerance, model } = req;
  const w = bitmap.width;
  const h = bitmap.height;
  if (!w || !h) throw new Error('empty image');

  const srcCanvas = new OffscreenCanvas(w, h);
  const srcCtx = srcCanvas.getContext('2d', { willReadFrequently: true });
  if (!srcCtx) throw new Error('OffscreenCanvas 2d context unavailable');
  srcCtx.drawImage(bitmap, 0, 0);
  bitmap.close();
  const imageData = srcCtx.getImageData(0, 0, w, h);
  const img: RawImage = { data: imageData.data, width: w, height: h };

  // a. 透明直通：已带 alpha 的图不再抠，RGB/alpha 原样进入后处理。
  let method: MatteMethod;
  let alpha: Uint8ClampedArray;
  if (transparentRatio(imageData.data) > TRANSPARENT_RATIO) {
    method = 'passthrough';
    alpha = new Uint8ClampedArray(w * h);
    for (let i = 0; i < alpha.length; i++) alpha[i] = imageData.data[i * 4 + 3];
  } else {
    // b. auto uses only the explicit model Blob; heuristic never initializes ONNX.
    if (mode === 'auto' && !model?.size) {
      emit({ id, type: 'fallback', reason: 'model_unavailable' });
      alpha = heuristicMatte(img, { tolerance });
      method = 'heuristic';
    } else if (mode === 'auto' && model) {
      try {
        alpha = await mlMatte(img, model, (phase) => emit({ id, type: 'progress', phase }));
        method = 'ml';
      } catch (err) {
        emit({ id, type: 'fallback', reason: 'inference_failed', detail: err instanceof Error ? err.message : String(err) });
        alpha = heuristicMatte(img, { tolerance });
        method = 'heuristic';
      }
    } else {
      alpha = heuristicMatte(img, { tolerance });
      method = 'heuristic';
    }

    // c. 失效检测：ML 失效降级启发式再试一次；启发式仍失效 → MATTE_FAILED。
    if (isMatteInvalid(alpha)) {
      if (method === 'ml') {
        emit({ id, type: 'fallback', reason: 'unsuitable_image' });
        alpha = heuristicMatte(img, { tolerance });
        method = 'heuristic';
      }
      if (isMatteInvalid(alpha)) throw new Error(MATTE_FAILED);
    }

    // d. 应用 alpha。
    for (let i = 0; i < alpha.length; i++) imageData.data[i * 4 + 3] = alpha[i];
    srcCtx.putImageData(imageData, 0, 0);
  }

  emit({ id, type: 'progress', phase: 'process' });

  // 包围盒（alpha > 10，外扩 8px）。
  let minX = w;
  let minY = h;
  let maxX = -1;
  let maxY = -1;
  for (let y = 0; y < h; y++) {
    const base = y * w;
    for (let x = 0; x < w; x++) {
      if (alpha[base + x] > BBOX_ALPHA_THRESHOLD) {
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
    }
  }
  if (maxX < 0) throw new Error(MATTE_FAILED); // 全透明
  const bx = Math.max(0, minX - BBOX_MARGIN);
  const by = Math.max(0, minY - BBOX_MARGIN);
  const bw = Math.min(w - 1, maxX + BBOX_MARGIN) - bx + 1;
  const bh = Math.min(h - 1, maxY + BBOX_MARGIN) - by + 1;

  // 长边 >2048 等比重采样。
  const scale = Math.min(1, MAX_OUTPUT_EDGE / Math.max(bw, bh));
  const fw = Math.max(1, Math.round(bw * scale));
  const fh = Math.max(1, Math.round(bh * scale));

  const outCanvas = new OffscreenCanvas(fw, fh);
  const outCtx = outCanvas.getContext('2d');
  if (!outCtx) throw new Error('OffscreenCanvas 2d context unavailable');
  outCtx.imageSmoothingEnabled = true;
  outCtx.imageSmoothingQuality = 'high';
  outCtx.drawImage(srcCanvas, bx, by, bw, bh, 0, 0, fw, fh);
  const blob = await outCanvas.convertToBlob({ type: 'image/webp', quality: 0.9 });

  // headBox 在裁切后的 alpha 上建议（比例坐标，缩放不变）。
  const cropAlpha = new Uint8ClampedArray(bw * bh);
  for (let y = 0; y < bh; y++) {
    const base = (by + y) * w + bx;
    cropAlpha.set(alpha.subarray(base, base + bw), y * bw);
  }
  const headBox = suggestHeadBox(cropAlpha, bw, bh);

  return { blob, aspect: fw / fh, headBox, method };
}
