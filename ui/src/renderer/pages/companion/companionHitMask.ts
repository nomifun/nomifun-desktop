/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 桌面伙伴「真实像素命中掩码」。
 *
 * 点击穿透原先按 data-companion-hit 元素的外接矩形命中，自定义立绘的矩形≈整窗，
 * 周围透明像素被当成「在立绘上」而捕获鼠标 → 点不到底层。这里给立绘元素挂一张由其
 * 透明 PNG 焼成的低分辨率 alpha 网格：光标落在透明像素上即放行穿透，落在实体上才命中。
 *
 * registry 用 WeakMap：元素卸载即随之回收，无需手动清理 key（仍提供 unregister 以便
 * 立绘换图时即时撤销旧掩码）。
 */

/** 命中采样器：元素本地归一坐标(0..1) → 是否落在非透明实体像素上（true=实体=命中）。 */
export type AlphaSampler = (x01: number, y01: number) => boolean;

const masks = new WeakMap<Element, AlphaSampler>();

export function registerAlphaMask(el: Element, sampler: AlphaSampler): void {
  masks.set(el, sampler);
}

export function unregisterAlphaMask(el: Element): void {
  masks.delete(el);
}

export function getAlphaMask(el: Element): AlphaSampler | undefined {
  return masks.get(el);
}

/**
 * 把一张已 CORS-clean 的透明立绘焼成低分辨率 alpha 网格并返回采样器。
 * - gridW×gridH：网格分辨率（64×96 足够区分立绘轮廓内外，内存 ~6KB）。
 * - threshold：alpha 阈值（>threshold 视为实体），默认 16 放行近全透明边缘。
 * 失败（无 2d 上下文 / getImageData 抛 SecurityError）返回 null，调用方回退矩形命中。
 */
export function buildAlphaSampler(
  img: HTMLImageElement,
  gridW = 64,
  gridH = 96,
  threshold = 16
): AlphaSampler | null {
  try {
    const canvas = document.createElement('canvas');
    canvas.width = gridW;
    canvas.height = gridH;
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(img, 0, 0, gridW, gridH);
    const rgba = ctx.getImageData(0, 0, gridW, gridH).data;
    // 只留 alpha 通道省内存。
    const alpha = new Uint8Array(gridW * gridH);
    for (let i = 0; i < gridW * gridH; i++) alpha[i] = rgba[i * 4 + 3];
    return (x01: number, y01: number): boolean => {
      if (x01 < 0 || x01 > 1 || y01 < 0 || y01 > 1) return false;
      const cx = Math.min(gridW - 1, Math.max(0, Math.round(x01 * (gridW - 1))));
      const cy = Math.min(gridH - 1, Math.max(0, Math.round(y01 * (gridH - 1))));
      // 3×3 邻域：吸收 mesh 摇摆(±~1.3px)与网格量化误差，立绘边缘不抖。
      for (let dy = -1; dy <= 1; dy++) {
        for (let dx = -1; dx <= 1; dx++) {
          const nx = cx + dx;
          const ny = cy + dy;
          if (nx < 0 || nx >= gridW || ny < 0 || ny >= gridH) continue;
          if (alpha[ny * gridW + nx] > threshold) return true;
        }
      }
      return false;
    };
  } catch {
    return null;
  }
}
