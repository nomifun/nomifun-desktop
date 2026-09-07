/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import {
  imageWorkbenchAspectRatioChoices,
  imageWorkbenchAspectRatioValue,
  imageWorkbenchResolutionLabel,
  imageWorkbenchResolutionOptions,
  imageWorkbenchSizeDimensionsLabel,
  imageWorkbenchSizeOptionForAspectRatio,
  imageWorkbenchSizeOptionForSettings,
  type ImageWorkbenchAspectRatioOption,
} from './types';
import styles from './ImageSizePicker.module.css';

/** Two independent UI controls resolving to one existing provider-safe size. */
const ImageSizePicker: React.FC<{
  options: readonly ImageWorkbenchAspectRatioOption[];
  value: string;
  disabled?: boolean;
  onChange(option: ImageWorkbenchAspectRatioOption): void;
}> = ({ options, value, disabled = false, onChange }) => {
  const { t } = useTranslation();
  const selected = imageWorkbenchSizeOptionForSettings(options, { aspectRatio: value });
  const aspectRatio = selected ? imageWorkbenchAspectRatioValue(selected) : '';
  const resolutions = imageWorkbenchResolutionOptions(options, aspectRatio);

  return (
    <div className={styles.picker} data-image-size-picker>
      <fieldset className={styles.group} disabled={disabled}>
        <legend>{t('creativeStudio.image.settings.aspectRatio', { defaultValue: '宽高比' })}</legend>
        <div className={styles.options}>
          {imageWorkbenchAspectRatioChoices(options).map((choice) => (
            <button
              key={choice.value}
              type='button'
              className={styles.option}
              aria-pressed={aspectRatio === choice.value}
              disabled={disabled || choice.disabled}
              onClick={() => {
                const next = imageWorkbenchSizeOptionForAspectRatio(options, selected, choice.value);
                if (next) onChange(next);
              }}
            >
              <span className={styles.shapeSlot} aria-hidden='true'>
                <span
                  className={styles.shape}
                  data-auto={choice.value === 'auto' || undefined}
                  style={choice.width && choice.height ? {
                    width: `${Math.min(18, 18 * choice.width / choice.height)}px`,
                    height: `${Math.min(18, 18 * choice.height / choice.width)}px`,
                  } : undefined}
                />
              </span>
              <span>{choice.label}</span>
            </button>
          ))}
        </div>
      </fieldset>
      <fieldset className={styles.group} disabled={disabled}>
        <legend>{t('creativeStudio.image.settings.resolution', { defaultValue: '分辨率' })}</legend>
        <div className={styles.options}>
          {resolutions.map((option) => (
            <button
              key={option.value}
              type='button'
              className={styles.option}
              aria-pressed={selected?.value === option.value}
              title={imageWorkbenchSizeDimensionsLabel(option) ?? undefined}
              disabled={disabled || option.disabled}
              onClick={() => onChange(option)}
            >
              {imageWorkbenchResolutionLabel(option)}
            </button>
          ))}
        </div>
        <div className={styles.dimensions} aria-live='polite'>
          {selected ? imageWorkbenchSizeDimensionsLabel(selected) : null}
        </div>
      </fieldset>
    </div>
  );
};

export default ImageSizePicker;
