/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useRef } from 'react';
import { useTranslation } from 'react-i18next';
import classNames from 'classnames';
import { bustCropStyle } from '@renderer/pages/companion/characters/customMeta';
import { FIGURE_HEIGHTS } from '@renderer/pages/companion/characters/customDesk';

export interface HeadBox {
  /** Left edge as a fraction of image width. */
  x: number;
  /** Top edge as a fraction of image height. */
  y: number;
  /** Box width as a fraction of image width. */
  w: number;
  /** Box height as a fraction of image height. */
  h: number;
}

/** Checkerboard backdrop so the transparent cutout reads as a cutout. */
export const CHECKER_BG: React.CSSProperties = {
  backgroundImage:
    'linear-gradient(45deg, rgba(0,0,0,0.10) 25%, transparent 25%, transparent 75%, rgba(0,0,0,0.10) 75%), linear-gradient(45deg, rgba(0,0,0,0.10) 25%, transparent 25%, transparent 75%, rgba(0,0,0,0.10) 75%)',
  backgroundSize: '14px 14px',
  backgroundPosition: '0 0, 7px 7px',
};

/** Frame-step viewport bounds (px). */
const MAX_VIEW_W = 320;
const MAX_VIEW_H = 300;
/** Bust preview side — must match the 84px picker-card avatar size. */
const PREVIEW_SIDE = 84;
/** Minimum head-box side as a fraction of the image, per axis. */
const MIN_SIDE = 0.08;
/** Tallest size-tier thumbnail (px); the other tiers scale proportionally. */
const SIZE_PREVIEW_MAX_H = 64;

const SIZE_TIERS: ReadonlyArray<'s' | 'm' | 'l'> = ['s', 'm', 'l'];
/** Static i18n keys (no dynamic key construction, so i18n typing stays happy). */
const SIZE_LABEL_KEY = {
  s: 'nomi.customFigure.sizeS',
  m: 'nomi.customFigure.sizeM',
  l: 'nomi.customFigure.sizeL',
} as const;

/**
 * Clamp a free-rectangle head box into the image. `x`/`w` are image-width
 * fractions, `y`/`h` are image-height fractions; each side is bounded to
 * [MIN_SIDE, 1] and the origin kept so the box never leaves the image.
 */
export function clampHeadBox(x: number, y: number, w: number, h: number): HeadBox {
  const cw = Math.min(Math.max(MIN_SIDE, w), 1);
  const ch = Math.min(Math.max(MIN_SIDE, h), 1);
  const cx = Math.min(Math.max(0, x), 1 - cw);
  const cy = Math.min(Math.max(0, y), 1 - ch);
  return { x: cx, y: cy, w: cw, h: ch };
}

interface FrameStepProps {
  /** Object URL of the matted cutout. */
  imageUrl: string;
  /** width / height of the cutout. */
  aspect: number;
  headBox: HeadBox;
  onHeadBoxChange: (hb: HeadBox) => void;
  sizeTier: 's' | 'm' | 'l';
  onSizeTierChange: (tier: 's' | 'm' | 'l') => void;
}

/**
 * Frame step: drag/resize a free-rectangle head-and-shoulders viewfinder over
 * the cutout (pure pointer events, no library) with a live 84px bust preview,
 * plus a size-tier picker whose three thumbnails render the figure at the tiers'
 * relative desktop heights so the size difference is visible at a glance. The
 * bust preview replicates CustomFigure's `bustCropStyle` (contain-fit, centered)
 * so what the user frames is exactly what the avatar shows.
 */
const FrameStep: React.FC<FrameStepProps> = ({ imageUrl, aspect, headBox, onHeadBoxChange, sizeTier, onSizeTierChange }) => {
  const { t } = useTranslation();

  const viewH = Math.min(MAX_VIEW_H, MAX_VIEW_W / aspect);
  const viewW = viewH * aspect;
  const boxLeft = headBox.x * viewW;
  const boxTop = headBox.y * viewH;
  const boxW = headBox.w * viewW;
  const boxH = headBox.h * viewH;

  const dragRef = useRef<{ mode: 'move' | 'resize'; px: number; py: number; hb: HeadBox } | null>(null);

  const startDrag = (mode: 'move' | 'resize') => (e: React.PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    e.currentTarget.setPointerCapture(e.pointerId);
    dragRef.current = { mode, px: e.clientX, py: e.clientY, hb: headBox };
  };
  const onPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const d = dragRef.current;
    if (!d) return;
    const dx = e.clientX - d.px;
    const dy = e.clientY - d.py;
    if (d.mode === 'move') {
      onHeadBoxChange(clampHeadBox(d.hb.x + dx / viewW, d.hb.y + dy / viewH, d.hb.w, d.hb.h));
    } else {
      // Free two-axis resize from the bottom-right handle; top-left stays anchored.
      onHeadBoxChange(clampHeadBox(d.hb.x, d.hb.y, d.hb.w + dx / viewW, d.hb.h + dy / viewH));
    }
  };
  const endDrag = () => {
    dragRef.current = null;
  };

  /* Bust preview: same contain-fit crop math CustomFigure uses for the avatar. */
  const preview = bustCropStyle(headBox, aspect, PREVIEW_SIDE);

  return (
    <div className='flex gap-20px items-start justify-center flex-wrap'>
      <div
        className='relative shrink-0 rd-8px overflow-hidden border border-solid border-[var(--color-border-2)]'
        style={{ width: viewW, height: viewH, ...CHECKER_BG }}
      >
        <img src={imageUrl} alt='' draggable={false} className='absolute inset-0 w-full h-full select-none' />
        <div
          onPointerDown={startDrag('move')}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          className='absolute border-2 border-solid border-[var(--color-primary)] rd-4px cursor-move'
          style={{
            left: boxLeft,
            top: boxTop,
            width: boxW,
            height: boxH,
            touchAction: 'none',
            boxShadow: '0 0 0 9999px rgba(0,0,0,0.35)',
          }}
        >
          <div
            onPointerDown={startDrag('resize')}
            onPointerMove={onPointerMove}
            onPointerUp={endDrag}
            onPointerCancel={endDrag}
            className='absolute -right-6px -bottom-6px w-12px h-12px rd-full bg-[var(--color-bg-1)] border-2 border-solid border-[var(--color-primary)] cursor-nwse-resize'
            style={{ touchAction: 'none' }}
          />
        </div>
      </div>

      <div className='flex flex-col gap-12px min-w-160px'>
        <span className='text-12px text-t-tertiary leading-snug max-w-200px'>{t('nomi.customFigure.frameHint')}</span>
        <div
          className='relative rd-12px overflow-hidden border border-solid border-[var(--color-border-2)]'
          style={{ width: PREVIEW_SIDE, height: PREVIEW_SIDE, ...CHECKER_BG }}
        >
          <img
            src={imageUrl}
            alt=''
            draggable={false}
            className='absolute max-w-none select-none'
            style={{ width: preview.width, height: preview.height, left: preview.left, top: preview.top }}
          />
        </div>
        <div className='flex flex-col gap-6px'>
          <div className='flex items-baseline gap-6px'>
            <span className='text-13px text-t-secondary'>{t('nomi.customFigure.sizeLabel')}</span>
            <span className='text-11px text-t-tertiary'>{t('nomi.customFigure.sizeHint')}</span>
          </div>
          <div className='flex gap-8px'>
            {SIZE_TIERS.map((tier) => {
              const ph = Math.round((FIGURE_HEIGHTS[tier] / FIGURE_HEIGHTS.l) * SIZE_PREVIEW_MAX_H);
              const selected = sizeTier === tier;
              return (
                <div
                  key={tier}
                  onClick={() => onSizeTierChange(tier)}
                  className={classNames(
                    'flex-1 flex flex-col items-center gap-3px pt-8px pb-6px rd-10px cursor-pointer transition-all duration-150 border border-solid',
                    selected ? 'border-[var(--color-primary)] bg-primary-1' : 'border-[var(--color-border-2)] bg-fill-1 hover:border-[var(--color-primary)]'
                  )}
                >
                  <div className='flex items-end justify-center w-full' style={{ height: SIZE_PREVIEW_MAX_H }}>
                    <img
                      src={imageUrl}
                      alt=''
                      draggable={false}
                      className='object-contain select-none'
                      style={{ height: ph, width: 'auto', maxWidth: '100%' }}
                    />
                  </div>
                  <span className={classNames('text-12px font-600', selected ? 'text-primary-6' : 'text-t-secondary')}>
                    {t(SIZE_LABEL_KEY[tier])}
                  </span>
                  <span className='text-10px text-t-tertiary'>{FIGURE_HEIGHTS[tier]}px</span>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
};

export default FrameStep;
