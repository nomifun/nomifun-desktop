/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * onnxruntime-web (WASM) ML 抠图：MODNet（Xenova/modnet ONNX 移植，Apache-2.0）。
 *
 * 模型分发：不打进安装包；首次使用从 MODEL_URL 懒下载（带进度），Cache Storage
 * 持久缓存（key 与 URL 解耦——换镜像源只改 MODEL_URL，缓存仍命中）。
 *
 * 预处理（已按 Xenova/modnet preprocessor_config.json 核实）：
 *   shortest_edge=512、size_divisibility=32、bilinear、
 *   rescale 1/255 + normalize mean/std 0.5 ⇔ (x - 127.5) / 127.5，NCHW Float32。
 * 输入/输出张量名运行时取 session.inputNames[0] / outputNames[0]（实际为
 * "input" / "output"，输出 [1,1,H,W] 的 0..1 matte），不硬编码。
 *
 * 本模块运行在 matting.worker 内：numThreads 固定 1，避免 ort 在我们的 Worker
 * 里再嵌套 spawn wasm 线程 Worker（也绕开 SharedArrayBuffer/COOP-COEP 要求）。
 * 模块顶层零副作用——模型下载与 ort env 配置全部惰性。
 */

// 用 /wasm 子入口（仅 wasm EP）：默认入口含 WebGPU/JSEP 构建，会让 Vite 多发射
// 一份 26MB 的 jsep wasm 资源；我们只用 wasm EP。
import * as ort from 'onnxruntime-web/wasm';
// Vite 把 wasm 资源当 asset 发射并返回最终 URL（包 exports 显式导出了该子路径）。
import ortWasmUrl from 'onnxruntime-web/ort-wasm-simd-threaded.wasm?url';

import type { RawImage } from './heuristicMatte';
import { MODEL_CACHE_NAME, modelCacheKey } from './modelCacheKey';

/** Sentinel: the model isn't in Cache Storage yet (the main thread owns the
 *  download via `modelCache.ensureMattingModel`). Callers fall back to heuristic. */
export const MODEL_NOT_READY = 'MODEL_NOT_READY';

/** MODNet 参考边（preprocessor: shortest_edge）。 */
const REF_EDGE = 512;
/** 极端长宽比时钳住长边，控制 wasm 推理成本（spec：1024 量级推理）。 */
const MAX_LONG_EDGE = 1024;
const SIZE_DIVISOR = 32;

// ---------------------------------------------------------------------------
// 模型读取（只读 Cache Storage）
// ---------------------------------------------------------------------------

/**
 * 取模型字节：**只**从 Cache Storage 读（主线程 `ensureMattingModel` 经后端代理
 * 预先写入）。命中直返；未命中抛 [`MODEL_NOT_READY`]——绝不在 worker 里直连远端
 * 下载（那正是 30s 超时死路的根因）。
 */
export async function fetchModel(): Promise<ArrayBuffer> {
  if (typeof caches === 'undefined') throw new Error(MODEL_NOT_READY);
  let cache: Cache;
  try {
    cache = await caches.open(MODEL_CACHE_NAME);
  } catch {
    throw new Error(MODEL_NOT_READY);
  }
  const hit = await cache.match(modelCacheKey());
  if (!hit) throw new Error(MODEL_NOT_READY);
  return await hit.arrayBuffer();
}

// ---------------------------------------------------------------------------
// 预处理 / 后处理（纯函数，零 DOM 依赖）
// ---------------------------------------------------------------------------

/**
 * 推理尺寸：短边对齐 512，长边钳 1024，两边取 32 的倍数（≥32）。
 * 与 Xenova/modnet processor（shortest_edge=512, size_divisibility=32）一致，
 * 仅多了极端长宽比下的长边钳制。
 */
export function computeInferenceSize(
  width: number,
  height: number
): { width: number; height: number } {
  const short = Math.min(width, height);
  const long = Math.max(width, height);
  let scale = REF_EDGE / short;
  scale = Math.min(scale, MAX_LONG_EDGE / long);
  const round32 = (v: number): number =>
    Math.max(SIZE_DIVISOR, Math.round(v / SIZE_DIVISOR) * SIZE_DIVISOR);
  return { width: round32(width * scale), height: round32(height * scale) };
}

/** 纯 TS 双线性 RGBA 缩放（OffscreenCanvas 不可用时的兜底；alpha 一并重采样）。 */
export function resizeRgbaBilinear(img: RawImage, dw: number, dh: number): RawImage {
  const { data, width: sw, height: sh } = img;
  const out = new Uint8ClampedArray(dw * dh * 4);
  const xRatio = sw / dw;
  const yRatio = sh / dh;
  for (let y = 0; y < dh; y++) {
    const sy = Math.min(sh - 1, Math.max(0, (y + 0.5) * yRatio - 0.5));
    const y0 = Math.floor(sy);
    const y1 = Math.min(sh - 1, y0 + 1);
    const fy = sy - y0;
    for (let x = 0; x < dw; x++) {
      const sx = Math.min(sw - 1, Math.max(0, (x + 0.5) * xRatio - 0.5));
      const x0 = Math.floor(sx);
      const x1 = Math.min(sw - 1, x0 + 1);
      const fx = sx - x0;
      const p00 = (y0 * sw + x0) * 4;
      const p01 = (y0 * sw + x1) * 4;
      const p10 = (y1 * sw + x0) * 4;
      const p11 = (y1 * sw + x1) * 4;
      const o = (y * dw + x) * 4;
      for (let c = 0; c < 4; c++) {
        const top = data[p00 + c] * (1 - fx) + data[p01 + c] * fx;
        const bottom = data[p10 + c] * (1 - fx) + data[p11 + c] * fx;
        out[o + c] = Math.round(top * (1 - fy) + bottom * fy);
      }
    }
  }
  return { data: out, width: dw, height: dh };
}

/** RGBA → NCHW Float32，(x - 127.5) / 127.5（= rescale 1/255 + mean/std 0.5）。 */
export function rgbaToNchwNormalized(img: RawImage): Float32Array {
  const { data, width, height } = img;
  const n = width * height;
  const out = new Float32Array(3 * n);
  for (let i = 0; i < n; i++) {
    const p = i * 4;
    out[i] = (data[p] - 127.5) / 127.5;
    out[n + i] = (data[p + 1] - 127.5) / 127.5;
    out[2 * n + i] = (data[p + 2] - 127.5) / 127.5;
  }
  return out;
}

/** 0..1 matte 双线性放大回原尺寸 → 0..255 alpha。 */
export function resizeMatteBilinear(
  matte: Float32Array,
  sw: number,
  sh: number,
  dw: number,
  dh: number
): Uint8ClampedArray {
  const out = new Uint8ClampedArray(dw * dh);
  const xRatio = sw / dw;
  const yRatio = sh / dh;
  for (let y = 0; y < dh; y++) {
    const sy = Math.min(sh - 1, Math.max(0, (y + 0.5) * yRatio - 0.5));
    const y0 = Math.floor(sy);
    const y1 = Math.min(sh - 1, y0 + 1);
    const fy = sy - y0;
    for (let x = 0; x < dw; x++) {
      const sx = Math.min(sw - 1, Math.max(0, (x + 0.5) * xRatio - 0.5));
      const x0 = Math.floor(sx);
      const x1 = Math.min(sw - 1, x0 + 1);
      const fx = sx - x0;
      const top = matte[y0 * sw + x0] * (1 - fx) + matte[y0 * sw + x1] * fx;
      const bottom = matte[y1 * sw + x0] * (1 - fx) + matte[y1 * sw + x1] * fx;
      out[y * dw + x] = Math.round((top * (1 - fy) + bottom * fy) * 255);
    }
  }
  return out;
}

/** 优先 OffscreenCanvas drawImage 缩放（Worker 原生、快）；不可用回退纯 TS 双线性。 */
function resizeRgba(img: RawImage, dw: number, dh: number): RawImage {
  if (typeof OffscreenCanvas !== 'undefined' && typeof ImageData !== 'undefined') {
    try {
      const src = new OffscreenCanvas(img.width, img.height);
      const sctx = src.getContext('2d');
      const dst = new OffscreenCanvas(dw, dh);
      const dctx = dst.getContext('2d', { willReadFrequently: true });
      if (sctx && dctx) {
        sctx.putImageData(new ImageData(img.data, img.width, img.height), 0, 0);
        dctx.imageSmoothingEnabled = true;
        dctx.imageSmoothingQuality = 'high';
        dctx.drawImage(src, 0, 0, img.width, img.height, 0, 0, dw, dh);
        const scaled = dctx.getImageData(0, 0, dw, dh);
        return { data: scaled.data, width: dw, height: dh };
      }
    } catch {
      // 回退纯 TS 路径
    }
  }
  return resizeRgbaBilinear(img, dw, dh);
}

// ---------------------------------------------------------------------------
// 推理
// ---------------------------------------------------------------------------

let ortEnvReady = false;
function ensureOrtEnv(): void {
  if (ortEnvReady) return;
  ortEnvReady = true;
  // bundle 变体（默认 import）已内嵌 .mjs 胶水，只需覆盖 .wasm 的取址。
  ort.env.wasm.wasmPaths = { wasm: ortWasmUrl };
  // 固定单线程：本模块已运行在专用 Worker 内，避免 ort 再嵌套 spawn Worker。
  ort.env.wasm.numThreads = 1;
}

/** 跨调用复用 session（"重抠"等场景免去重复初始化 25MB 模型）；失败则清缓存。 */
let sessionPromise: Promise<ort.InferenceSession> | undefined;
function getSession(): Promise<ort.InferenceSession> {
  if (!sessionPromise) {
    sessionPromise = (async () => {
      ensureOrtEnv();
      // 只读 Cache Storage（主线程已预先写入）；未命中抛 MODEL_NOT_READY。
      const modelBuf = await fetchModel();
      return await ort.InferenceSession.create(modelBuf, { executionProviders: ['wasm'] });
    })();
    sessionPromise.catch(() => {
      sessionPromise = undefined;
    });
  }
  return sessionPromise;
}

/**
 * MODNet ML 抠图：返回原尺寸 alpha 通道（width*height 字节）。模型只从 Cache
 * Storage 读取（主线程负责下载）；未就绪/推理失败直接抛出，由调用方决定降级。
 */
export async function mlMatte(
  img: RawImage,
  onProgress?: (phase: 'infer') => void
): Promise<Uint8ClampedArray> {
  const session = await getSession();
  onProgress?.('infer');

  const { width: iw, height: ih } = computeInferenceSize(img.width, img.height);
  const resized = iw === img.width && ih === img.height ? img : resizeRgba(img, iw, ih);
  const input = new ort.Tensor('float32', rgbaToNchwNormalized(resized), [1, 3, ih, iw]);

  // run 抛错可能意味着 session 已毒化（wasm 状态损坏）——清缓存让下次重建（审查 M4）。
  let results: Awaited<ReturnType<typeof session.run>>;
  try {
    results = await session.run({ [session.inputNames[0]]: input });
  } catch (err) {
    sessionPromise = undefined;
    throw err;
  }
  const output = results[session.outputNames[0]];
  if (!output) throw new Error('model returned no output');
  const dims = output.dims;
  const ow = dims.length >= 2 ? Number(dims[dims.length - 1]) : iw;
  const oh = dims.length >= 2 ? Number(dims[dims.length - 2]) : ih;
  const matte = output.data as Float32Array;
  if (matte.length < ow * oh) throw new Error('unexpected matte tensor shape');

  return resizeMatteBilinear(matte, ow, oh, img.width, img.height);
}
