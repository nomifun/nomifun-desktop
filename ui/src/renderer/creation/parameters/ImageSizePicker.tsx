/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import {
  imageGenerationAspectRatioChoices,
  imageGenerationAspectRatioValue,
  imageGenerationResolutionLabel,
  imageGenerationResolutionOptions,
  imageGenerationSizeDimensionsLabel,
  imageGenerationSizeOptionForAspectRatio,
  imageGenerationSizeOptionForSettings,
  type ImageGenerationAspectRatioOption,
} from './image';
import styles from './ImageSizePicker.module.css';

/** Two independent UI controls resolving to one existing provider-safe size. */
const ImageSizePicker: React.FC<{
  options: readonly ImageGenerationAspectRatioOption[];
  value: string;
  disabled?: boolean;
  onChange(option: ImageGenerationAspectRatioOption): void;
}> = ({ options, value, disabled = false, onChange }) => {
  const { t } = useTranslation();
  const selected = imageGenerationSizeOptionForSettings(options, { aspectRatio: value });
  const aspectRatio = selected ? imageGenerationAspectRatioValue(selected) : '';
  const resolutions = imageGenerationResolutionOptions(options, aspectRatio);

  return (
    <div className={styles.picker} data-image-size-picker>
      <fieldset className={styles.group} disabled={disabled}>
        <legend>{t('creativeStudio.image.settings.aspectRatio', { defaultValue: '宽高比' })}</legend>
        <div className={styles.options}>
          {imageGenerationAspectRatioChoices(options).map((choice) => (
            <button
              key={choice.value}
              type='button'
              className={styles.option}
              aria-pressed={aspectRatio === choice.value}
              disabled={disabled || choice.disabled}
              onClick={() => {
                const next = imageGenerationSizeOptionForAspectRatio(options, selected, choice.value);
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
              title={imageGenerationSizeDimensionsLabel(option) ?? undefined}
              disabled={disabled || option.disabled}
              onClick={() => onChange(option)}
            >
              {imageGenerationResolutionLabel(option)}
            </button>
          ))}
        </div>
        <div className={styles.dimensions} aria-live='polite'>
          {selected ? imageGenerationSizeDimensionsLabel(selected) : null}
        </div>
      </fieldset>
    </div>
  );
};

export default ImageSizePicker;
