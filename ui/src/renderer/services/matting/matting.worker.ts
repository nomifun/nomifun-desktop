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
 * 消息协议：
 *   in:  { id: number; type: 'matte'; bitmap: ImageBitmap; mode: 'auto' | 'heuristic'; tolerance?: number }
 *   out: { id; type: 'progress'; phase: 'download' | 'infer' | 'process'; loaded?: number; total?: number }
 *        { id; type: 'done'; blob: Blob; aspect: number; headBox: { x; y; w }; method: 'passthrough' | 'ml' | 'heuristic' }
 *        { id; type: 'error'; message: string }   // message='MATTE_FAILED' 表示抠图失效，i18n 在 UI 层做
 */

import { heuristicMatte, suggestHeadBox, type RawImage } from './heuristicMatte';
import { mlMatte } from './onnxMatte';

export interface MatteRequest {
  id: number;
  type: 'matte';
  bitmap: ImageBitmap;
  mode: 'auto' | 'heuristic';
  tolerance?: number;
}

export type MatteMethod = 'passthrough' | 'ml' | 'heuristic';

export type MatteResponse =
  | {
      id: number;
      type: 'progress';
      phase: 'download' | 'infer' | 'process';
      loaded?: number;
      total?: number;
    }
  | {
      id: number;
      type: 'done';
      blob: Blob;
      aspect: number;
      headBox: { x: number; y: number; w: number };
      method: MatteMethod;
    }
  | { id: number; type: 'error'; message: string };

/** 抠图失效错误码（UI 层译为"无法识别背景，请提供透明底图或更简单背景"）。 */
const MATTE_FAILED = 'MATTE_FAILED';

const ML_TIMEOUT_MS = 30_000;
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

const post = (msg: MatteResponse): void => self.postMessage(msg);

self.onmessage = (ev) => {
  const req = ev.data;
  if (!req || req.type !== 'matte') return;
  void handle(req);
};

async function handle(req: MatteRequest): Promise<void> {
  const { id } = req;
  try {
    const result = await matte(req);
    post({ id, type: 'done', ...result });
  } catch (err) {
    post({ id, type: 'error', message: err instanceof Error ? err.message : String(err) });
  }
}

function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('ML matting timed out')), ms);
    promise.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        clearTimeout(timer);
        reject(e);
      }
    );
  });
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

async function matte(req: MatteRequest): Promise<{
  blob: Blob;
  aspect: number;
  headBox: { x: number; y: number; w: number };
  method: MatteMethod;
}> {
  const { id, bitmap, mode, tolerance } = req;
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
    // b. auto 先 ML（模型未就绪/推理异常/超时 → 降级启发式）；heuristic 直走泛洪。
    //    模型由主线程经后端代理预先写入 Cache Storage，worker 只读不下载——故此超时
    //    只覆盖「读缓存 + 建 session + 推理」（秒级），不再像旧实现那样把 25MB 下载
    //    也套进 30s 里（那正是「首次必超时 → 死在抠图步」的根因）。
    if (mode === 'auto') {
      try {
        alpha = await withTimeout(
          mlMatte(img, (phase) => post({ id, type: 'progress', phase })),
          ML_TIMEOUT_MS
        );
        method = 'ml';
      } catch (err) {
        // 降级不可见会掩盖网络/模型问题（审查 I1）——留一条 warn 便于排障。
        console.warn('[matting] ML failed, falling back to heuristic:', err);
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
        alpha = heuristicMatte(img, { tolerance });
        method = 'heuristic';
      }
      if (isMatteInvalid(alpha)) throw new Error(MATTE_FAILED);
    }

    // d. 应用 alpha。
    for (let i = 0; i < alpha.length; i++) imageData.data[i * 4 + 3] = alpha[i];
    srcCtx.putImageData(imageData, 0, 0);
  }

  post({ id, type: 'progress', phase: 'process' });

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
