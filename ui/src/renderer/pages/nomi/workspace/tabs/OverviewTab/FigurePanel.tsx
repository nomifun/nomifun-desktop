/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Slider } from '@arco-design/web-react';
import type { ICompanionProfile } from '@/common/adapter/ipcBridge';
import { CUSTOM_CHARACTER_ID } from '@renderer/pages/companion/characters';
import type { CustomFigureMeta } from '@renderer/pages/companion/characters';
import { FIGURE_HEIGHTS, SIZE_MAX, SIZE_MIN } from '@renderer/pages/companion/characters/customDesk';
import CharacterPicker from '@renderer/pages/nomi/CharacterPicker';
import { figureToCustomPatch } from '@renderer/pages/nomi/useFigures';
import type { CompanionHandle } from '../../types';
import RowAction from './RowAction';

interface FigurePanelProps {
  profile: ICompanionProfile;
  patchCompanion: CompanionHandle['patchCompanion'];
  /** Parsed `appearance.custom_figure`; null for a built-in character. */
  figure: CustomFigureMeta | null;
}

/**
 * Contents of the 更换形象 detail pane: pick a look, then (for a DIY figure) tune
 * how tall it stands on the desktop. It lives in the pane rather than a modal so
 * the user can keep adjusting while watching the actual desktop companion resize —
 * every commit broadcasts `companion.config-updated`, which the pet window applies
 * live.
 */
const FigurePanel: React.FC<FigurePanelProps> = ({ profile, patchCompanion, figure }) => {
  const { t } = useTranslation();

  const effectiveHeight = figure ? (figure.sizePx ?? FIGURE_HEIGHTS[figure.sizeTier]) : FIGURE_HEIGHTS.m;
  const [sizeDraft, setSizeDraft] = useState<number>(effectiveHeight);
  // Re-sync the slider whenever the persisted value (or the selected figure) moves.
  useEffect(() => {
    setSizeDraft(effectiveHeight);
  }, [effectiveHeight]);

  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  /** Size a scheduled-but-not-yet-fired commit would write; null when idle. */
  const pendingRef = useRef<number | null>(null);
  const commitRef = useRef<(size_px: number | null) => void>(() => {});

  // RFC 7396 merge patch over the existing wire `custom_figure`: a number sets the
  // override, null deletes it and falls back to the figure's size tier.
  const commitSize = useCallback(
    (size_px: number | null) => {
      const base = profile.appearance.custom_figure;
      if (!base) return;
      void patchCompanion({ appearance: { custom_figure: { ...base, size_px } } });
    },
    [patchCompanion, profile.appearance.custom_figure]
  );
  commitRef.current = commitSize;

  // This panel lives in a pane the user closes with a click, so a drag released
  // <400ms before that click would otherwise be thrown away. Flush it instead —
  // `commitRef` still holds the closure bound to the companion being edited.
  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
      const pending = pendingRef.current;
      pendingRef.current = null;
      if (pending !== null) commitRef.current(pending);
    },
    []
  );

  const onSizeChange = useCallback(
    (value: number | number[]) => {
      const next = Array.isArray(value) ? value[0] : value;
      setSizeDraft(next);
      pendingRef.current = next;
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        pendingRef.current = null;
        commitSize(next);
      }, 400);
    },
    [commitSize]
  );

  const onSizeReset = useCallback(() => {
    if (timerRef.current) clearTimeout(timerRef.current);
    pendingRef.current = null;
    commitSize(null);
  }, [commitSize]);

  return (
    <div className='flex flex-col gap-14px'>
      <CharacterPicker
        compact
        value={profile.character || 'mochi'}
        figureId={figure?.figureId}
        onSelectCharacter={(character) => void patchCompanion({ character, appearance: { custom_figure: null } })}
        onSelectFigure={(fig) =>
          void patchCompanion({
            character: CUSTOM_CHARACTER_ID,
            appearance: { custom_figure: figureToCustomPatch(fig) },
          })
        }
      />

      {figure && (
        <div className='flex flex-col gap-8px rd-10px border border-solid border-[var(--color-border-2)] px-12px py-10px'>
          <div className='flex items-center justify-between gap-8px'>
            <span className='text-13px font-500 text-t-primary'>
              {t('nomi.customFigure.sizeLabel', { defaultValue: '桌面形象尺寸' })}
            </span>
            <span className='shrink-0 text-12px text-t-primary'>{`${sizeDraft}px`}</span>
          </div>
          <div className='flex items-center gap-8px'>
            <span className='shrink-0 text-11px text-t-tertiary'>
              {t('nomi.customFigure.sizeS', { defaultValue: '小' })}
            </span>
            <Slider className='min-w-0 flex-1' min={SIZE_MIN} max={SIZE_MAX} step={4} value={sizeDraft} onChange={onSizeChange} />
            <span className='shrink-0 text-11px text-t-tertiary'>
              {t('nomi.customFigure.sizeL', { defaultValue: '大' })}
            </span>
          </div>
          <div className='flex items-center justify-between gap-8px'>
            <span className='min-w-0 text-11px leading-16px text-t-tertiary'>
              {t('nomi.customFigure.sizeHint', { defaultValue: '桌面显示高度' })}
            </span>
            {figure.sizePx != null && (
              <RowAction quiet onClick={onSizeReset}>
                {t('nomi.customFigure.sizeReset', { defaultValue: '复位' })}
              </RowAction>
            )}
          </div>
        </div>
      )}
    </div>
  );
};

export default FigurePanel;
