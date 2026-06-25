/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 启发式泛洪抠图（移植自已在真实立绘上验证过的 Python 原型）：
 * 1. BFS 背景泛洪：四边边框像素作种子，邻居与当前像素的 RGB Chebyshev
 *    距离 ≤ tolerance 则并入背景。
 * 2. 回捞：被吞的像素若色度 (max-min) ≥ 16 或亮度 max ≥ 118 改回前景
 *    （仅图内非边框区——边框始终视为背景）。亮度判据只在深色背景下启用
 *    （边框环最大通道中位数 < 118），浅色背景下会整片误捞、故禁用。
 * 3. 连通域清理：前景 4-连通块面积 < 30px 一律丢弃；面积 < 120px 且块内
 *    平均色度 < 25 也丢弃（保留小而鲜艳的粒子）。
 * 4. 羽化：二值 alpha 做一次 3×3 盒式模糊（边界 1px 内产生过渡）。
 *
 * 纯函数、零 DOM 依赖——Worker 与单测通用。
 */

export interface RawImage {
  // Always ArrayBuffer-backed (canvas `getImageData` / fresh `new Uint8ClampedArray`,
  // never a SharedArrayBuffer), so the explicit param keeps the `ImageData`
  // constructor + canvas APIs in lib.es2024 happy without weakening any byte.
  data: Uint8ClampedArray<ArrayBuffer>; // RGBA
  width: number;
  height: number;
}

const DEFAULT_TOLERANCE = 11;
const RESTORE_CHROMA = 16;
const RESTORE_BRIGHTNESS = 118;
const MIN_COMPONENT_AREA = 30;
const SMALL_COMPONENT_AREA = 120;
const SMALL_COMPONENT_MIN_CHROMA = 25;
const HEAD_ALPHA_THRESHOLD = 40;
const HEAD_BOX_FRACTION = 0.3;

/** Returns the computed alpha channel (width*height bytes). Pure function — no DOM. */
export function heuristicMatte(img: RawImage, opts?: { tolerance?: number }): Uint8ClampedArray {
  const { data, width: w, height: h } = img;
  const tol = opts?.tolerance ?? DEFAULT_TOLERANCE;
  const n = w * h;
  if (n === 0) return new Uint8ClampedArray(0);

  // --- 1) BFS 背景泛洪（边框种子，局部连续性生长） ---
  const bg = new Uint8Array(n); // 1 = background
  const queue = new Int32Array(n); // 每像素至多入队一次
  let head = 0;
  let tail = 0;
  const seed = (i: number): void => {
    if (!bg[i]) {
      bg[i] = 1;
      queue[tail++] = i;
    }
  };
  for (let x = 0; x < w; x++) {
    seed(x);
    seed((h - 1) * w + x);
  }
  for (let y = 0; y < h; y++) {
    seed(y * w);
    seed(y * w + w - 1);
  }
  // 4-连通邻居偏移：右、左、下、上（dx 用于行边界判断）。
  while (head < tail) {
    const i = queue[head++];
    const x = i % w;
    const p = i * 4;
    const r = data[p];
    const g = data[p + 1];
    const b = data[p + 2];
    for (let k = 0; k < 4; k++) {
      let ni: number;
      if (k === 0) {
        if (x + 1 >= w) continue;
        ni = i + 1;
      } else if (k === 1) {
        if (x - 1 < 0) continue;
        ni = i - 1;
      } else if (k === 2) {
        if (i + w >= n) continue;
        ni = i + w;
      } else {
        if (i - w < 0) continue;
        ni = i - w;
      }
      if (bg[ni]) continue;
      const q = ni * 4;
      if (
        Math.abs(data[q] - r) <= tol &&
        Math.abs(data[q + 1] - g) <= tol &&
        Math.abs(data[q + 2] - b) <= tol
      ) {
        bg[ni] = 1;
        queue[tail++] = ni;
      }
    }
  }

  // --- 2) 回捞被误吞的前景（色度或亮度判据；边框 1px 不参与） ---
  // 「亮即前景」判据隐含背景为深色的前提：背景本身就亮（边框环最大通道
  // 中位数 ≥ RESTORE_BRIGHTNESS）时禁用之，否则整片白底/浅灰底会被整体
  // 回捞成前景（前景占比爆表 → Worker 端 MATTE_FAILED）。色度判据不受影响。
  const maxCh = (i: number): number => Math.max(data[i * 4], data[i * 4 + 1], data[i * 4 + 2]);
  const borderMax: number[] = [];
  for (let x = 0; x < w; x++) {
    borderMax.push(maxCh(x), maxCh((h - 1) * w + x));
  }
  for (let y = 1; y < h - 1; y++) {
    borderMax.push(maxCh(y * w), maxCh(y * w + w - 1));
  }
  borderMax.sort((a, b) => a - b);
  const lightBackground = borderMax[borderMax.length >> 1] >= RESTORE_BRIGHTNESS;
  for (let y = 1; y < h - 1; y++) {
    for (let x = 1; x < w - 1; x++) {
      const i = y * w + x;
      if (!bg[i]) continue;
      const p = i * 4;
      const r = data[p];
      const g = data[p + 1];
      const b = data[p + 2];
      const mx = Math.max(r, g, b);
      const mn = Math.min(r, g, b);
      if (mx - mn >= RESTORE_CHROMA || (!lightBackground && mx >= RESTORE_BRIGHTNESS)) bg[i] = 0;
    }
  }

  // --- 3) 连通域清理：丢弃小面积低色度碎块（保留小而鲜艳的粒子） ---
  const visited = new Uint8Array(n);
  for (let s = 0; s < n; s++) {
    if (bg[s] || visited[s]) continue;
    // 复用 queue 作为本块的 BFS 队列；queue[0..tail) 即该连通块成员。
    head = 0;
    tail = 0;
    visited[s] = 1;
    queue[tail++] = s;
    let chromaSum = 0;
    while (head < tail) {
      const i = queue[head++];
      const x = i % w;
      const p = i * 4;
      const r = data[p];
      const g = data[p + 1];
      const b = data[p + 2];
      chromaSum += Math.max(r, g, b) - Math.min(r, g, b);
      if (x + 1 < w && !bg[i + 1] && !visited[i + 1]) {
        visited[i + 1] = 1;
        queue[tail++] = i + 1;
      }
      if (x - 1 >= 0 && !bg[i - 1] && !visited[i - 1]) {
        visited[i - 1] = 1;
        queue[tail++] = i - 1;
      }
      if (i + w < n && !bg[i + w] && !visited[i + w]) {
        visited[i + w] = 1;
        queue[tail++] = i + w;
      }
      if (i - w >= 0 && !bg[i - w] && !visited[i - w]) {
        visited[i - w] = 1;
        queue[tail++] = i - w;
      }
    }
    const area = tail;
    const meanChroma = chromaSum / area;
    if (
      area < MIN_COMPONENT_AREA ||
      (area < SMALL_COMPONENT_AREA && meanChroma < SMALL_COMPONENT_MIN_CHROMA)
    ) {
      for (let k = 0; k < tail; k++) bg[queue[k]] = 1;
    }
  }

  // --- 4) 羽化：二值 alpha 的 3×3 盒式模糊（图像边缘按实有邻居归一化） ---
  const alpha = new Uint8ClampedArray(n);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let sum = 0;
      let count = 0;
      for (let dy = -1; dy <= 1; dy++) {
        const ny = y + dy;
        if (ny < 0 || ny >= h) continue;
        for (let dx = -1; dx <= 1; dx++) {
          const nx = x + dx;
          if (nx < 0 || nx >= w) continue;
          if (!bg[ny * w + nx]) sum += 255;
          count++;
        }
      }
      alpha[y * w + x] = Math.round(sum / count);
    }
  }
  return alpha;
}

/** Suggest a square head box from an alpha channel: top content row + horizontal
 *  centroid of the top 30% of content rows; side = 30% of image width clamped into bounds.
 *  Returns image-fraction coords { x, y, w }. */
export function suggestHeadBox(
  alpha: Uint8ClampedArray,
  width: number,
  height: number
): { x: number; y: number; w: number } {
  const side = Math.min(Math.max(1, Math.round(width * HEAD_BOX_FRACTION)), width, height);
  const w = width > 0 ? side / width : HEAD_BOX_FRACTION;
  // 内容行 = 该行存在 alpha > 40 的像素。
  let yTop = -1;
  let yBottom = -1;
  for (let y = 0; y < height; y++) {
    const base = y * width;
    for (let x = 0; x < width; x++) {
      if (alpha[base + x] > HEAD_ALPHA_THRESHOLD) {
        if (yTop < 0) yTop = y;
        yBottom = y;
        break;
      }
    }
  }
  if (yTop < 0) return { x: (1 - w) / 2, y: 0, w }; // 全透明：顶部居中兜底
  // 顶部 30% 内容高度范围内的 x 质心。
  const contentHeight = yBottom - yTop + 1;
  const yEnd = Math.min(yBottom, yTop + Math.max(1, Math.ceil(contentHeight * 0.3)) - 1);
  let sumX = 0;
  let count = 0;
  for (let y = yTop; y <= yEnd; y++) {
    const base = y * width;
    for (let x = 0; x < width; x++) {
      if (alpha[base + x] > HEAD_ALPHA_THRESHOLD) {
        sumX += x;
        count++;
      }
    }
  }
  const cx = count > 0 ? sumX / count : width / 2;
  const xPx = Math.min(Math.max(0, cx - side / 2), width - side);
  const yPx = Math.min(yTop, Math.max(0, height - side));
  return { x: xPx / width, y: yPx / height, w };
}
