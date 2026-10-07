/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { Slider } from '@arco-design/web-react';
import { IconMinus, IconPlus, IconRefresh } from '@arco-design/web-react/icon';
import { useTranslation } from 'react-i18next';
import { useThemeContext } from '@renderer/hooks/context/ThemeContext';
import { FONT_SCALE_DEFAULT, FONT_SCALE_MAX, FONT_SCALE_MIN, FONT_SCALE_STEP } from '@renderer/hooks/ui/useFontScale';

const EPSILON = 0.001;
const RESET_THRESHOLD = 0.01;
const clamp = (value: number) => Math.min(FONT_SCALE_MAX, Math.max(FONT_SCALE_MIN, value));

/** Stepper, editable percentage and slider share the existing persisted zoom. */
const FontSizeControl: React.FC = () => {
  const { t } = useTranslation();
  const { fontScale, setFontScale } = useThemeContext();
  const percentage = String(Math.round(fontScale * 100));
  const [inputValue, setInputValue] = useState(percentage);

  useEffect(() => setInputValue(percentage), [percentage]);

  // Commit on blur/Enter so typing a percentage does not resize the UI mid-edit.
  const commitInput = () => {
    if (!/^\d+$/.test(inputValue.trim())) {
      setInputValue(percentage);
      return;
    }
    const stepPercent = FONT_SCALE_STEP * 100;
    const next = clamp((Math.round(Number(inputValue) / stepPercent) * stepPercent) / 100);
    setInputValue(String(Math.round(next * 100)));
    if (Math.abs(next - fontScale) > EPSILON) void setFontScale(next);
  };

  const defaultMarks = useMemo(
    () => ({ 1: <span className='font-scale-default-mark' aria-hidden='true' title='100%' /> }),
    []
  );
  const handleSliderChange = (value: number | number[]) => {
    if (typeof value === 'number') void setFontScale(clamp(Number(value.toFixed(2))));
  };
  const handleStep = (delta: number) => void setFontScale(clamp(Number((fontScale + delta).toFixed(2))));
  const isResetDisabled = Math.abs(fontScale - FONT_SCALE_DEFAULT) < RESET_THRESHOLD;

  return (
    <div className='font-scale-control'>
      <div className='font-scale-control__header'>
        <span>{t('settings.fontSize')}</span>
        <div className='font-scale-control__actions'>
          <div className='font-scale-control__stepper'>
            <button
              type='button'
              className='font-scale-control__step'
              aria-label={t('settings.fontSizeDecrease')}
              onClick={() => handleStep(-FONT_SCALE_STEP)}
              disabled={fontScale <= FONT_SCALE_MIN + EPSILON}
            >
              <IconMinus aria-hidden='true' />
            </button>
            <label className='font-scale-control__value'>
              <input
                type='text'
                inputMode='numeric'
                aria-label={t('settings.fontSize')}
                value={inputValue}
                maxLength={3}
                onChange={(event) => setInputValue(event.target.value)}
                onBlur={commitInput}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    event.preventDefault();
                    event.currentTarget.blur();
                  } else if (event.key === 'Escape' && inputValue !== percentage) {
                    event.preventDefault();
                    event.stopPropagation();
                    setInputValue(percentage);
                  }
                }}
              />
              <span aria-hidden='true'>%</span>
            </label>
            <button
              type='button'
              className='font-scale-control__step'
              aria-label={t('settings.fontSizeIncrease')}
              onClick={() => handleStep(FONT_SCALE_STEP)}
              disabled={fontScale >= FONT_SCALE_MAX - EPSILON}
            >
              <IconPlus aria-hidden='true' />
            </button>
          </div>
          <button
            type='button'
            className='font-scale-control__reset'
            aria-label={t('settings.fontSizeReset')}
            title={t('settings.fontSizeReset')}
            onClick={() => void setFontScale(FONT_SCALE_DEFAULT)}
            disabled={isResetDisabled}
          >
            <IconRefresh aria-hidden='true' />
          </button>
        </div>
      </div>
      <Slider
        className='font-scale-slider'
        aria-label={t('settings.fontSize')}
        min={FONT_SCALE_MIN}
        max={FONT_SCALE_MAX}
        step={FONT_SCALE_STEP}
        value={fontScale}
        onChange={handleSliderChange}
        marks={defaultMarks}
      />
    </div>
  );
};

export default FontSizeControl;
