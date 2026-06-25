/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { processCustomCss } from '@renderer/utils/theme/customCssProcessor';

/**
 * Inject the active ambiance-preset CSS into a STANDALONE window that does not
 * mount {@link Layout} (the desktop companion). Mirrors Layout's injection (same
 * `<style id>`, same `!important` wrapping via {@link processCustomCss}, appended
 * last in `<head>` so it wins the cascade), with one companion-specific guard:
 *
 * Ambiance presets paint `body { background-color: … }` for the main app's
 * backdrop. The companion window is a transparent, always-on-top overlay — that
 * solid body background would turn it into an opaque rectangle. So we append a
 * transparency guard AFTER the preset rules (same specificity + importance, so
 * last-wins) to keep `html`/`body`/`#root` see-through; the visible chrome is
 * drawn by companion.css and still picks up the preset's CSS variables.
 *
 * Passing an empty string removes the injected style entirely.
 */
const STYLE_ID = 'user-defined-custom-css';
// 透明保护：把 html/body/#root 钉成透明，挡住氛围预设画在 body 上的实底天幕。
// 关键 specificity：预设的暗色背景挂在 `[data-theme='dark'] body`(0,1,1)，经 processCustomCss
// 加 !important 后会压过裸 `body`(0,0,1) 的 guard —— 所以 guard 必须覆盖到同级/更高
// specificity 的 `[data-theme] body` / `html[data-theme] body`，靠 source order 在后取胜。
// 并显式清 background-image/background-color（预设用 background-image 画 radial/linear，单写
// background:transparent 在部分 WebView2 下不一定清干净 image）。
const TRANSPARENCY_GUARD =
  '\nhtml, body, #root,' +
  '\n[data-theme] html, [data-theme] body, [data-theme] #root,' +
  '\nhtml[data-theme] body, html[data-theme] #root' +
  '\n{ background: transparent !important; background-image: none !important; background-color: transparent !important; }';

export function injectCompanionCustomCss(css: string): void {
  if (typeof document === 'undefined') return;

  const existing = document.getElementById(STYLE_ID);
  if (!css || !css.trim()) {
    existing?.remove();
    return;
  }

  const content = processCustomCss(css) + TRANSPARENCY_GUARD;

  // Idempotent + keep-last: if our style is already the final <head> child with
  // identical content, leave it; otherwise (re)append so it outranks the base
  // palette and any arco styles injected after mount.
  if (existing && existing.textContent === content && existing === document.head.lastElementChild) {
    return;
  }
  existing?.remove();

  const styleEl = document.createElement('style');
  styleEl.id = STYLE_ID;
  styleEl.type = 'text/css';
  styleEl.textContent = content;
  document.head.appendChild(styleEl);
}
