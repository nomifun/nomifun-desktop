/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, it } from 'vitest';
import { heuristicMatte, suggestHeadBox, type RawImage } from './heuristicMatte';

function makeImage(width: number, height: number): RawImage {
  return { data: new Uint8ClampedArray(width * height * 4), width, height };
}

function setPx(img: RawImage, x: number, y: number, r: number, g: number, b: number): void {
  const i = (y * img.width + x) * 4;
  img.data[i] = r;
  img.data[i + 1] = g;
  img.data[i + 2] = b;
  img.data[i + 3] = 255;
}

/**
 * 12×12 灰渐变底（水平步进 4 / 垂直步进 2，均 ≤ tolerance=11，泛洪可全吞；
 * 最大灰度 106 < 118 且色度 0 < 16，不会被回捞）+ 中央 6×6 红块（36px ≥ 30，
 * 不被连通域清理；色度 160 与底色差 > tolerance，泛洪进不来）。
 */
function makeGradientScene(): RawImage {
  const img = makeImage(12, 12);
  for (let y = 0; y < 12; y++) {
    for (let x = 0; x < 12; x++) {
      const v = 40 + 4 * x + 2 * y;
      setPx(img, x, y, v, v, v);
    }
  }
  for (let y = 3; y <= 8; y++) {
    for (let x = 3; x <= 8; x++) {
      setPx(img, x, y, 200, 40, 40);
    }
  }
  return img;
}

describe('heuristicMatte', () => {
  it('floods the gradient background and keeps the central red block', () => {
    const img = makeGradientScene();
    const alpha = heuristicMatte(img, { tolerance: 11 });

    // 红块内部（6×6 块的内 4×4，避开羽化过渡环）保持前景。
    for (let y = 4; y <= 7; y++) {
      for (let x = 4; x <= 7; x++) {
        expect(alpha[y * 12 + x]).toBeGreaterThanOrEqual(200);
      }
    }
    // 四角背景被泛洪清空。
    for (const [x, y] of [[0, 0], [11, 0], [0, 11], [11, 11]] as const) {
      expect(alpha[y * 12 + x]).toBeLessThanOrEqual(20);
    }
    // 羽化：紧贴红块上缘外侧 1px 处应是过渡值（无羽化则为 0，全糊则 ≥200）。
    expect(alpha[2 * 12 + 5]).toBeGreaterThan(40);
    expect(alpha[2 * 12 + 5]).toBeLessThan(200);
  });

  it('drops an isolated low-chroma speck via connected-component cleanup', () => {
    const img = makeGradientScene();
    // 孤立 1×2 灰点：色度 0 < 16、亮度 100 < 118（回捞不会救它），
    // 与周边渐变（60~72）差 ≥ 28 > tolerance（泛洪绕过它留为前景），
    // 面积 2 < 30 → 连通域清理丢弃。
    setPx(img, 1, 10, 100, 100, 100);
    setPx(img, 2, 10, 100, 100, 100);
    const alpha = heuristicMatte(img, { tolerance: 11 });

    expect(alpha[10 * 12 + 1]).toBeLessThanOrEqual(20);
    expect(alpha[10 * 12 + 2]).toBeLessThanOrEqual(20);
    // 清理不可误伤真正的前景。
    expect(alpha[5 * 12 + 5]).toBeGreaterThanOrEqual(200);
  });

  it('restores a flood-swallowed dark-red band by chroma', () => {
    // 14×14 平灰底 + 8×8 暗红条带（x/y 3..10）：按到块边距离 d 设色
    // (68+7d, 52-7d, 52-7d)——每步 Chebyshev 差 ≤ 8 ≤ tolerance，泛洪整块吞掉；
    // 各环色度 16/30/44/58 ≥ 16 → 回捞；面积 64 ≥ 30 且均值色度 28.25 ≥ 25 → 不被清理。
    const img = makeImage(14, 14);
    for (let y = 0; y < 14; y++) {
      for (let x = 0; x < 14; x++) {
        setPx(img, x, y, 60, 60, 60);
      }
    }
    for (let y = 3; y <= 10; y++) {
      for (let x = 3; x <= 10; x++) {
        const d = Math.min(x - 3, 10 - x, y - 3, 10 - y);
        setPx(img, x, y, 68 + 7 * d, 52 - 7 * d, 52 - 7 * d);
      }
    }
    const alpha = heuristicMatte(img, { tolerance: 11 });

    // 条带内部被回捞为前景（避开羽化过渡环只看内 4×4）。
    for (let y = 5; y <= 8; y++) {
      for (let x = 5; x <= 8; x++) {
        expect(alpha[y * 14 + x]).toBeGreaterThanOrEqual(200);
      }
    }
    // 平灰背景仍是背景。
    expect(alpha[0]).toBeLessThanOrEqual(20);
    expect(alpha[13 * 14 + 13]).toBeLessThanOrEqual(20);
  });

  it('keeps a solid light-gray background as background (no brightness re-capture)', () => {
    // 14×14 纯浅灰底 (232) + 中央 6×6 红块：泛洪吞掉均匀浅底后，
    // 「亮即前景」回捞判据（max ≥ 118）若不感知背景明暗，会把整片内部
    // 浅底回捞成前景 → 前景占比爆表（Worker 端 MATTE_FAILED）。
    // 期望：浅底保持背景，红块保持前景（白底/浅灰底是 DIY 上传最常见场景）。
    const img = makeImage(14, 14);
    for (let y = 0; y < 14; y++) {
      for (let x = 0; x < 14; x++) {
        setPx(img, x, y, 232, 232, 232);
      }
    }
    for (let y = 4; y <= 9; y++) {
      for (let x = 4; x <= 9; x++) {
        setPx(img, x, y, 200, 40, 40);
      }
    }
    const alpha = heuristicMatte(img, { tolerance: 11 });

    // 红块内部（内 4×4，避开羽化过渡环）保持前景。
    for (let y = 5; y <= 8; y++) {
      for (let x = 5; x <= 8; x++) {
        expect(alpha[y * 14 + x]).toBeGreaterThanOrEqual(200);
      }
    }
    // 内部浅底（非边框 1px 区）不被亮度判据回捞。
    expect(alpha[1 * 14 + 1]).toBeLessThanOrEqual(20);
    expect(alpha[12 * 14 + 12]).toBeLessThanOrEqual(20);
    expect(alpha[2 * 14 + 11]).toBeLessThanOrEqual(20);
  });

  it('still restores swallowed bright low-chroma foreground on a dark background', () => {
    // 23×23 深灰底 (90) + 15×15 灰块（x/y 4..18）：按环距 d 设灰度 101+11d
    // ——每步 11 ≤ tolerance，泛洪整块吞掉；色度 0 救不回，only 亮度判据：
    // d≥2 的 11×11=121px 内核 ≥ 123 ≥ 118 → 回捞，且 121 ≥ 120 不被
    // 小面积低色度清理。深底图的亮度回捞行为必须保持不变。
    const img = makeImage(23, 23);
    for (let y = 0; y < 23; y++) {
      for (let x = 0; x < 23; x++) {
        setPx(img, x, y, 90, 90, 90);
      }
    }
    for (let y = 4; y <= 18; y++) {
      for (let x = 4; x <= 18; x++) {
        const d = Math.min(x - 4, 18 - x, y - 4, 18 - y);
        const v = Math.min(255, 101 + 11 * d);
        setPx(img, x, y, v, v, v);
      }
    }
    const alpha = heuristicMatte(img, { tolerance: 11 });

    // 内核中心（避开羽化过渡环）被亮度判据回捞为前景。
    for (let y = 9; y <= 13; y++) {
      for (let x = 9; x <= 13; x++) {
        expect(alpha[y * 23 + x]).toBeGreaterThanOrEqual(200);
      }
    }
    // 深灰背景仍是背景。
    expect(alpha[0]).toBeLessThanOrEqual(20);
    expect(alpha[22 * 23 + 22]).toBeLessThanOrEqual(20);
  });
});

describe('suggestHeadBox', () => {
  it('boxes the top content centroid at 30% of image width', () => {
    // 20×40：前景集中在 x 6..14、y 4..16。
    const width = 20;
    const height = 40;
    const alpha = new Uint8ClampedArray(width * height);
    for (let y = 4; y <= 16; y++) {
      for (let x = 6; x <= 14; x++) {
        alpha[y * width + x] = 255;
      }
    }
    const box = suggestHeadBox(alpha, width, height);

    expect(box.w).toBeCloseTo(0.3, 5);
    // y = 顶部内容行 4 / 高度 40。
    expect(box.y).toBeCloseTo(4 / 40, 5);
    // x 质心 10：side = 6px，框左缘 = (10 - 3)/20 = 0.35，且框必须覆盖质心。
    expect(box.x).toBeCloseTo(0.35, 2);
    expect(box.x).toBeLessThanOrEqual(10 / width);
    expect(box.x + box.w).toBeGreaterThanOrEqual(10 / width);
  });
});
