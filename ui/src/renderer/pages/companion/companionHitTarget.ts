/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { getAlphaMask } from './companionHitMask';

export interface CompanionHitStyle {
  pointerEvents?: string;
  visibility?: string;
  display?: string;
}

export interface CompanionHitTargetOptions {
  tolerancePx: number;
  getStyle?: (el: HTMLElement) => CompanionHitStyle | null | undefined;
}

const defaultGetStyle = (el: HTMLElement): CompanionHitStyle | null => {
  if (typeof window === 'undefined' || typeof window.getComputedStyle !== 'function') return null;
  return window.getComputedStyle(el);
};

const canReceivePointer = (el: HTMLElement, getStyle: (el: HTMLElement) => CompanionHitStyle | null | undefined): boolean => {
  const style = getStyle(el);
  if (!style) return true;
  return style.pointerEvents !== 'none' && style.visibility !== 'hidden' && style.display !== 'none';
};

export function isPointOverCompanionHitTarget(
  clientX: number,
  clientY: number,
  els: Iterable<HTMLElement>,
  opts: CompanionHitTargetOptions
): boolean {
  const getStyle = opts.getStyle ?? defaultGetStyle;
  for (const el of els) {
    if (!canReceivePointer(el, getStyle)) continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) continue;
    if (
      clientX >= r.left - opts.tolerancePx &&
      clientX <= r.right + opts.tolerancePx &&
      clientY >= r.top - opts.tolerancePx &&
      clientY <= r.bottom + opts.tolerancePx
    ) {
      const mask = getAlphaMask(el);
      if (mask) {
        const x01 = (clientX - r.left) / r.width;
        const y01 = (clientY - r.top) / r.height;
        if (!mask(x01, y01)) continue;
      }
      return true;
    }
  }
  return false;
}
