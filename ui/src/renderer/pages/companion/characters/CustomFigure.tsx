/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect } from 'react';
import type { CompanionActivity, CompanionMood } from './types';
import { bustCropStyle } from './customMeta';
import { registerAlphaMask, unregisterAlphaMask, buildAlphaSampler } from '../companionHitMask';

/**
 * CustomFigure — generic single-image full-body figure (a transparent cutout
 * artwork) rendered as a STATIC <img> with CSS-only animation (breathing + mood
 * posture). Any preset or user-uploaded single-image figure renders here.
 *
 * Why static <img> + CSS and NOT a WebGL mesh: the companion lives in a
 * transparent, always-on-top window. A per-frame requestAnimationFrame WebGL
 * canvas there becomes its own hardware compositing layer that DWM must
 * recomposite against the desktop every frame; the WebGL / Chromium-compositor
 * / DWM clocks are not lock-stepped, so DWM periodically samples a half-drawn
 * intermediate frame → the figure FLICKERS in every scene (idle AND drag). This
 * is a platform-level transparent-window + GPU-canvas conflict (Tauri
 * #15490/#14831, marked status:upstream; Electron historically needs
 * disableHardwareAcceleration for transparent windows). preserveDrawingBuffer
 * could not fix it — the flicker lives in the compositing chain, not the
 * drawing buffer. Built-in CSS/SVG characters in the SAME window never flicker,
 * which proves the WebGL canvas layer itself was the source. So the custom
 * figure now renders exactly like them: a static <img> the compositor
 * interpolates on its own clock — zero per-frame alpha surface, zero flicker.
 * CSS keyframes carry breathing + hop/jump/doze/worry; particle fx stay SVG.
 *
 * Modes:
 *  - full figure (size > BUST_MAX_SIZE): <img> + particle fx, CSS breathing.
 *  - head-and-shoulders (size ≤ BUST_MAX_SIZE): <img> cropped to headBox.
 */

export interface CustomFigureProps {
  /** Image URL of the transparent full-body cutout. */
  src: string;
  /** Image aspect (width / height). */
  aspect: number;
  /**
   * Head-and-shoulders crop in image-fraction coords: left x + width w (image
   * width), top y + height h (image height). Free rectangle; a legacy square box
   * has h = w·aspect.
   */
  headBox: { x: number; y: number; w: number; h: number };
  mood: CompanionMood;
  activity: CompanionActivity;
  size?: number;
  /** 命中元素（外层 data-companion-hit wrapper）；加载后在其上注册立绘 alpha 掩码。 */
  hitRef?: React.RefObject<HTMLElement | null>;
}

/** At or below this `size` the full figure is unreadable — render the bust crop. */
const BUST_MAX_SIZE = 130;

const CFIG_CSS = `
.nomi-cfig { position: relative; line-height: 0; }
.nomi-cfig img { display: block; -webkit-user-drag: none; }

/* ground shadow, pulsing against the breath */
.nomi-cfig__shadow {
  position: absolute; left: 27%; bottom: 0.2%; width: 46%; height: 3.2%;
  background: radial-gradient(closest-side, rgba(0,0,0,.16), rgba(0,0,0,0));
  animation: nomi-cfig-shadow 3.6s ease-in-out infinite;
}
@keyframes nomi-cfig-shadow { 0%,100% { transform: scaleX(1); opacity: 1; } 50% { transform: scaleX(.95); opacity: .72; } }

/* figure wrapper rig — CSS-only breathing + mood posture (no WebGL: a per-frame
   GPU canvas flickers in the transparent always-on-top window; see file header) */
.nomi-cfig__all { position: relative; width: 100%; height: 100%; transform-origin: 50% 100%; animation: nomi-cfig-breathe 3.6s ease-in-out infinite; }
@keyframes nomi-cfig-breathe { 0%,100% { transform: scaleY(1); } 50% { transform: scaleY(1.008) translateY(-1px); } }
.nomi-cfig--happy .nomi-cfig__all { animation: nomi-cfig-hop 1.7s ease-in-out infinite; }
@keyframes nomi-cfig-hop {
  0%,100% { transform: translateY(0); }
  30% { transform: translateY(-7px); }
  55% { transform: translateY(0) scaleY(.997); }
}
.nomi-cfig--excited .nomi-cfig__all { animation: nomi-cfig-jump 1.15s ease-in-out infinite; }
@keyframes nomi-cfig-jump {
  0%,100% { transform: translateY(0); }
  40% { transform: translateY(-12px); }
  62% { transform: translateY(-8px); }
  82% { transform: translateY(1px) scaleY(.995); }
}
.nomi-cfig--sleepy .nomi-cfig__all { animation: nomi-cfig-doze 4.6s ease-in-out infinite; }
@keyframes nomi-cfig-doze { 0%,100% { transform: translateY(1.5px) rotate(.35deg); } 50% { transform: translateY(3px) rotate(-.35deg); } }
.nomi-cfig--worried .nomi-cfig__all { animation: nomi-cfig-worry 2.6s ease-in-out infinite; }
@keyframes nomi-cfig-worry { 0%,100% { transform: translateY(2px) scaleY(.998); } 50% { transform: translateY(3px) scaleY(1.002); } }

.nomi-cfig__img { position: absolute; inset: 0; width: 100%; height: 100%; }

/* bust crop is a fixed camera window — light breathing only, never translate */
.nomi-cfig__bust-img { position: absolute; max-width: none; transform-origin: 50% 100%; animation: nomi-cfig-breathe 3.6s ease-in-out infinite; }

/* particle fx overlay (viewBox 0 0 944 1000) */
.nomi-cfig__fx { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; overflow: visible; }
.nomi-cfig__fx-el { transform-box: fill-box; transform-origin: 50% 50%; }
.nomi-cfig__z, .nomi-cfig__sweat, .nomi-cfig__spark, .nomi-cfig__leaf { opacity: 0; }
.nomi-cfig--sleepy .nomi-cfig__z1 { animation: nomi-cfig-z 3s ease-in-out infinite; }
.nomi-cfig--sleepy .nomi-cfig__z2 { animation: nomi-cfig-z 3s ease-in-out infinite 1.5s; }
@keyframes nomi-cfig-z {
  0% { opacity: 0; transform: translate(0,0) scale(.6); }
  25% { opacity: .9; }
  70% { opacity: .65; }
  100% { opacity: 0; transform: translate(32px,-68px) scale(1.1); }
}
.nomi-cfig--worried .nomi-cfig__sweat { animation: nomi-cfig-sweat 2.6s ease-in-out infinite; }
@keyframes nomi-cfig-sweat {
  0%,14% { opacity: 0; transform: translateY(0); }
  24% { opacity: .85; }
  72% { opacity: .85; }
  100% { opacity: 0; transform: translateY(45px); }
}
.nomi-cfig--excited .nomi-cfig__spark1 { animation: nomi-cfig-spark 1.1s ease-in-out infinite; }
.nomi-cfig--excited .nomi-cfig__spark2 { animation: nomi-cfig-spark 1.1s ease-in-out infinite .38s; }
.nomi-cfig--excited .nomi-cfig__spark3 { animation: nomi-cfig-spark 1.1s ease-in-out infinite .72s; }
@keyframes nomi-cfig-spark { 0%,100% { opacity: 0; transform: scale(.4); } 45% { opacity: 1; transform: scale(1.1); } }
.nomi-cfig--thinking .nomi-cfig__leaf1 { animation: nomi-cfig-leaf 3.4s ease-in-out infinite; }
.nomi-cfig--thinking .nomi-cfig__leaf2 { animation: nomi-cfig-leaf 3.4s ease-in-out infinite 1.7s; }
@keyframes nomi-cfig-leaf {
  0% { opacity: 0; transform: translate(23px,-46px) rotate(0deg); }
  12% { opacity: .95; }
  50% { transform: translate(-18px,82px) rotate(160deg); opacity: .9; }
  82% { opacity: .75; }
  100% { opacity: 0; transform: translate(18px,205px) rotate(330deg); }
}
`;

/** Four-point sparkle path centered at (cx, cy): outer radius R, waist r. */
const starPath = (cx: number, cy: number, R: number, r: number): string =>
  `M ${cx} ${cy - R} L ${cx + r} ${cy - r} L ${cx + R} ${cy} L ${cx + r} ${cy + r} ` +
  `L ${cx} ${cy + R} L ${cx - r} ${cy + r} L ${cx - R} ${cy} L ${cx - r} ${cy - r} Z`;

const CustomFigure: React.FC<CustomFigureProps> = ({ src, aspect, headBox, mood, activity, size = 150, hitRef }) => {
  const bust = size <= BUST_MAX_SIZE;

  // 加载立绘并在命中元素上注册 alpha 命中掩码：点击穿透按真实非透明像素判定，立绘四周
  // 透明区真正穿透到底层（见 companionHitMask）。仅全身态注册；bust 态小、矩形命中足够。
  // 不再启动 WebGL mesh —— 透明置顶窗里每帧重绘 GPU canvas 会被 DWM 逐帧重合成导致
  // 全场景闪烁（见组件顶部注释），改纯静态 <img> + CSS 动画彻底消闪。
  useEffect(() => {
    if (bust || !hitRef) return undefined;
    let disposed = false;
    let maskEl: Element | null = null;
    const img = new Image();
    // DIY 立绘从后端 origin(http://127.0.0.1:{port}) 加载、页面在壳 origin；后端回
    // ACAO:* → CORS-clean，getImageData 不会污染画布。同源(web SPA 相对 URL)下 anonymous 是 no-op。
    img.crossOrigin = 'anonymous';
    img.onload = () => {
      if (disposed) return;
      const target = hitRef.current;
      if (!target) return;
      const sampler = buildAlphaSampler(img);
      if (sampler) {
        registerAlphaMask(target, sampler);
        maskEl = target;
      }
    };
    img.src = src;
    return () => {
      disposed = true;
      if (maskEl) {
        unregisterAlphaMask(maskEl);
        maskEl = null;
      }
    };
  }, [bust, src, hitRef]);

  const w = bust ? size : Math.round(size * aspect);
  const h = size;

  /* bust crop: contain-fit the framed (possibly rectangular) head box into the
     square size×size avatar slot, centered — see bustCropStyle. */
  const bustStyle = bustCropStyle(headBox, aspect, size);

  return (
    <div
      className={`nomi-ch nomi-cfig nomi-cfig--${mood} nomi-cfig--${activity}${bust ? ' nomi-cfig--bust' : ''}`}
      style={{ width: w, height: h, overflow: bust ? 'hidden' : 'visible' }}
    >
      <style>{CFIG_CSS}</style>
      {bust ? (
        <img
          className='nomi-cfig__bust-img'
          src={src}
          alt=''
          draggable={false}
          style={{
            width: bustStyle.width,
            height: bustStyle.height,
            left: bustStyle.left,
            top: bustStyle.top,
          }}
        />
      ) : (
        <>
          <div className='nomi-cfig__shadow' />
          <div className='nomi-cfig__all'>
            <img className='nomi-cfig__img' src={src} alt='' draggable={false} />
          </div>

          {/* particle fx (outside the rig so hops don't fling them) */}
          <svg className='nomi-cfig__fx' viewBox='0 0 944 1000' aria-hidden='true'>
            <g fontFamily='Georgia, serif' fontWeight='700' fill='#b78757'>
              <text className='nomi-cfig__fx-el nomi-cfig__z nomi-cfig__z1' x='660' y='90' fontSize='40'>z</text>
              <text className='nomi-cfig__fx-el nomi-cfig__z nomi-cfig__z2' x='720' y='55' fontSize='54'>z</text>
            </g>
            <path
              className='nomi-cfig__fx-el nomi-cfig__sweat'
              d='M640 95 q 14 18 0 30 q -14 -12 0 -30 Z'
              fill='#9ed4f2'
              stroke='#6fb6dd'
              strokeWidth='3'
            />
            <g fill='#ffd23f'>
              <path className='nomi-cfig__fx-el nomi-cfig__spark nomi-cfig__spark1' d={starPath(180, 330, 26, 7)} />
              <path className='nomi-cfig__fx-el nomi-cfig__spark nomi-cfig__spark2' d={starPath(800, 430, 21, 6)} />
              <path className='nomi-cfig__fx-el nomi-cfig__spark nomi-cfig__spark3' d={starPath(150, 700, 17, 5)} />
            </g>
            <g className='nomi-cfig__fx-el nomi-cfig__leaf nomi-cfig__leaf1'>
              <path d='M560 60 q 25 -18 45 0 q -20 25 -45 0 Z' fill='#c84a3a' stroke='#9c3528' strokeWidth='3' />
            </g>
            <g className='nomi-cfig__fx-el nomi-cfig__leaf nomi-cfig__leaf2'>
              <path d='M360 90 q 20 -15 38 0 q -17 21 -38 0 Z' fill='#d8693a' stroke='#a8462a' strokeWidth='2.5' />
            </g>
          </svg>
        </>
      )}
    </div>
  );
};

export default CustomFigure;
