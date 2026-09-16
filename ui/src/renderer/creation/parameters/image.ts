/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { getI18n } from 'react-i18next';

export type ImageGenerationInterfaceMode = 'images' | 'responses';
export type ImageGenerationQuality = 'auto' | 'high' | 'medium' | 'low';

/** Exact NomiFun model identity. Display labels never become request identity. */
export interface ImageGenerationModelIdentity {
  providerId: string;
  model: string;
}

export interface ImageGenerationModelOption extends ImageGenerationModelIdentity {
  /** User-facing alias, falling back to the exact model id. */
  label: string;
  /** Exact id shown as secondary text when `label` is a configured alias. */
  rawModelId?: string;
  providerLabel?: string;
  /** Catalog protocol metadata used to select a provider-safe size policy. */
  platform?: string;
  protocol?: string;
  disabled?: boolean;
}

export interface ImageGenerationAspectRatioOption {
  value: string;
  label: string;
  /** Presentation metadata only; value/requestSize remain the saved/wire identity. */
  aspectRatio?: string;
  resolution?: string;
  width: number | null;
  height: number | null;
  /**
   * Exact provider-native size value. This is intentionally separate from
   * width/height because some providers use a different ordering or a fixed
   * enumeration for their wire contract.
   */
  requestSize?: string;
  disabled?: boolean;
}

/** Presentation-only ratio group derived from provider-safe exact size options. */
export interface ImageGenerationAspectRatioChoice {
  value: string;
  label: string;
  width: number | null;
  height: number | null;
  disabled: boolean;
}

export interface ImageGenerationSizePolicy {
  options: readonly ImageGenerationAspectRatioOption[];
  allowCustomDimensions: boolean;
  maxCount: number;
}

export interface ImageGenerationSettings {
  model: ImageGenerationModelIdentity | null;
  interfaceMode: ImageGenerationInterfaceMode;
  quality: ImageGenerationQuality;
  width: number | null;
  height: number | null;
  aspectRatio: string;
  count: number;
}

const localizedOptionLabel = (key: string, defaultValue: string): string =>
  getI18n()?.t(key, { defaultValue }) ?? defaultValue;

export const DEFAULT_IMAGE_GENERATION_ASPECT_RATIOS: readonly ImageGenerationAspectRatioOption[] = [
  { value: '1:1', label: '1:1', width: 1024, height: 1024, requestSize: '1024x1024' },
  { value: '3:2', label: '3:2', width: 1536, height: 1024, requestSize: '1536x1024' },
  { value: '2:3', label: '2:3', width: 1024, height: 1536, requestSize: '1024x1536' },
  { value: '4:3', label: '4:3', width: 1024, height: 768, requestSize: '1024x768' },
  { value: '3:4', label: '3:4', width: 768, height: 1024, requestSize: '768x1024' },
  { value: '16:9', label: '16:9', width: 1920, height: 1080, requestSize: '1920x1080' },
  { value: '9:16', label: '9:16', width: 1080, height: 1920, requestSize: '1080x1920' },
  { value: '21:9', label: '21:9', width: 1568, height: 672, requestSize: '1568x672' },
  { value: '2048x2048', label: '1:1 · 2K', aspectRatio: '1:1', resolution: '2K', width: 2048, height: 2048, requestSize: '2048x2048' },
  { value: '2048x1152', label: '16:9 · 2K', aspectRatio: '16:9', resolution: '2K', width: 2048, height: 1152, requestSize: '2048x1152' },
  { value: '1152x2048', label: '9:16 · 2K', aspectRatio: '9:16', resolution: '2K', width: 1152, height: 2048, requestSize: '1152x2048' },
  { value: '3840x2160', label: '16:9 · 4K', aspectRatio: '16:9', resolution: '4K', width: 3840, height: 2160, requestSize: '3840x2160' },
  { value: '2160x3840', label: '9:16 · 4K', aspectRatio: '9:16', resolution: '4K', width: 2160, height: 3840, requestSize: '2160x3840' },
  {
    value: 'auto',
    get label() {
      return localizedOptionLabel('creativeStudio.image.options.auto', '自动');
    },
    width: null,
    height: null,
  },
];

export function imageGenerationSizeDimensionsLabel(
  option: Pick<ImageGenerationAspectRatioOption, 'width' | 'height'>
): string | null {
  return option.width !== null && option.height !== null
    ? `${option.width} × ${option.height}`
    : null;
}

/** Group by explicit metadata or the existing ratio key, never translated labels. */
export function imageGenerationAspectRatioValue(
  option: ImageGenerationAspectRatioOption
): string {
  return option.aspectRatio ?? option.value;
}

function imageGenerationResolutionValue(option: ImageGenerationAspectRatioOption): string {
  return option.value === 'auto' ? 'auto' : option.resolution ?? 'standard';
}

/** The existing base presets vary in pixels; do not misrepresent them all as 1K. */
export function imageGenerationResolutionLabel(
  option: ImageGenerationAspectRatioOption
): string {
  const resolution = imageGenerationResolutionValue(option);
  if (resolution === 'auto') return option.label;
  if (resolution === 'standard') {
    return localizedOptionLabel('creativeStudio.image.settings.standardResolution', '标准');
  }
  return /^\d+$/.test(resolution) ? `${resolution} px` : resolution;
}

export function imageGenerationResolutionOptionLabel(
  option: ImageGenerationAspectRatioOption
): string {
  const dimensions = imageGenerationSizeDimensionsLabel(option);
  const resolution = imageGenerationResolutionLabel(option);
  return dimensions ? `${resolution} · ${dimensions}` : resolution;
}

export function imageGenerationAspectRatioChoices(
  options: readonly ImageGenerationAspectRatioOption[]
): ImageGenerationAspectRatioChoice[] {
  const choices = new Map<string, ImageGenerationAspectRatioChoice>();
  for (const option of options) {
    const value = imageGenerationAspectRatioValue(option);
    const current = choices.get(value);
    if (!current) {
      choices.set(value, {
        value,
        label: value === 'auto' ? option.label : value,
        width: option.width,
        height: option.height,
        disabled: Boolean(option.disabled),
      });
      continue;
    }
    if (current.disabled && !option.disabled) {
      choices.set(value, {
        ...current,
        width: option.width,
        height: option.height,
        disabled: false,
      });
    }
  }
  return [...choices.values()];
}

export function imageGenerationResolutionOptions(
  options: readonly ImageGenerationAspectRatioOption[],
  aspectRatio: string
): ImageGenerationAspectRatioOption[] {
  return options.filter(
    (option) => imageGenerationAspectRatioValue(option) === aspectRatio
  );
}

/**
 * Pick the exact provider option for a ratio change while retaining the current
 * resolution tier when that ratio supports it.
 */
export function imageGenerationSizeOptionForAspectRatio(
  options: readonly ImageGenerationAspectRatioOption[],
  current: ImageGenerationAspectRatioOption | null,
  aspectRatio: string
): ImageGenerationAspectRatioOption | null {
  const candidates = imageGenerationResolutionOptions(options, aspectRatio).filter(
    (option) => !option.disabled
  );
  if (candidates.length === 0) return null;
  if (current) {
    const resolution = imageGenerationResolutionValue(current);
    const sameResolution = candidates.find(
      (option) => imageGenerationResolutionValue(option) === resolution
    );
    if (sameResolution) return sameResolution;
  }
  return candidates[0] ?? null;
}

export function imageGenerationFixedSizeOptions(
  options: readonly ImageGenerationAspectRatioOption[]
): ImageGenerationAspectRatioOption[] {
  return options.filter(
    (option) => option.width !== null && option.height !== null
  );
}

export function imageGenerationSelectableSizeOptions(
  options: readonly ImageGenerationAspectRatioOption[]
): ImageGenerationAspectRatioOption[] {
  return options.filter(
    (option) =>
      (option.width !== null && option.height !== null) || option.value === 'auto'
  );
}

export function imageGenerationSizeOptionForSettings(
  options: readonly ImageGenerationAspectRatioOption[],
  settings: Pick<ImageGenerationSettings, 'aspectRatio'>
): ImageGenerationAspectRatioOption | null {
  return (
    options.find(
      (option) =>
        !option.disabled &&
        option.value === settings.aspectRatio
    ) ??
    options.find((option) => !option.disabled) ??
    null
  );
}

/** Width and height remain transport/history fields, but the selected size
 * option is their sole product-level authority. */
export function normalizeImageGenerationSettingsSize(
  settings: ImageGenerationSettings,
  policy: Pick<ImageGenerationSizePolicy, 'options' | 'maxCount'>
): ImageGenerationSettings {
  const option = imageGenerationSizeOptionForSettings(policy.options, settings);
  if (!option) return settings;
  const count = Math.min(settings.count, policy.maxCount);
  if (
    settings.aspectRatio === option.value &&
    settings.width === option.width &&
    settings.height === option.height &&
    settings.count === count
  ) {
    return settings;
  }
  return {
    ...settings,
    aspectRatio: option.value,
    width: option.width,
    height: option.height,
    count,
  };
}

/**
 * StepFun's image API uses a strict size enum. For step-image-edit-2 the
 * non-square wire value is height x width, while the workbench always keeps
 * width/height in the user-facing order.
 */
const STEPFUN_IMAGE_EDIT_2_ASPECT_RATIOS: readonly ImageGenerationAspectRatioOption[] = [
  {
    value: '1:1',
    label: '1:1',
    width: 1024,
    height: 1024,
    requestSize: '1024x1024',
  },
  {
    value: '16:9',
    label: '16:9',
    width: 1360,
    height: 768,
    requestSize: '768x1360',
  },
  {
    value: '4:3',
    label: '4:3',
    width: 1184,
    height: 896,
    requestSize: '896x1184',
  },
  {
    value: '9:16',
    label: '9:16',
    width: 768,
    height: 1360,
    requestSize: '1360x768',
  },
  {
    value: '3:4',
    label: '3:4',
    width: 896,
    height: 1184,
    requestSize: '1184x896',
  },
  {
    value: 'auto',
    get label() {
      return localizedOptionLabel('creativeStudio.image.options.auto', '自动');
    },
    width: null,
    height: null,
  },
];

const STEPFUN_STEP_2X_LARGE_ASPECT_RATIOS: readonly ImageGenerationAspectRatioOption[] = [
  {
    value: '1:1-256',
    label: '1:1 · 256',
    aspectRatio: '1:1',
    resolution: '256',
    width: 256,
    height: 256,
    requestSize: '256x256',
  },
  {
    value: '1:1-512',
    label: '1:1 · 512',
    aspectRatio: '1:1',
    resolution: '512',
    width: 512,
    height: 512,
    requestSize: '512x512',
  },
  {
    value: '1:1',
    label: '1:1',
    width: 1024,
    height: 1024,
    requestSize: '1024x1024',
  },
  {
    value: '16:9',
    label: '16:9',
    width: 1280,
    height: 800,
    requestSize: '1280x800',
  },
  {
    value: '9:16',
    label: '9:16',
    width: 800,
    height: 1280,
    requestSize: '800x1280',
  },
  {
    value: 'auto',
    get label() {
      return localizedOptionLabel('creativeStudio.image.options.auto', '自动');
    },
    width: null,
    height: null,
  },
];

const STEPFUN_UNKNOWN_MODEL_ASPECT_RATIOS: readonly ImageGenerationAspectRatioOption[] = [
  DEFAULT_IMAGE_GENERATION_ASPECT_RATIOS[0],
  DEFAULT_IMAGE_GENERATION_ASPECT_RATIOS[DEFAULT_IMAGE_GENERATION_ASPECT_RATIOS.length - 1],
];

export function imageGenerationSizePolicyForModel(
  model:
    | Pick<ImageGenerationModelOption, 'model' | 'platform' | 'protocol'>
    | null
    | undefined
): ImageGenerationSizePolicy {
  if (!model) {
    return {
      options: DEFAULT_IMAGE_GENERATION_ASPECT_RATIOS,
      allowCustomDimensions: true,
      maxCount: 10,
    };
  }

  const protocol = model.protocol?.trim().toLowerCase();
  const modelId = model.model.trim().toLowerCase();
  const isStepFunImages = protocol === 'stepfun.images';
  if (protocol === 'ark.images') {
    return {
      options: DEFAULT_IMAGE_GENERATION_ASPECT_RATIOS,
      allowCustomDimensions: true,
      // Ark's `count` is not an `n` parameter. Group output is a separate,
      // non-exact sequential-generation contract, so one task promises one
      // image unless that contract is modeled explicitly in the future.
      maxCount: 1,
    };
  }
  if (!isStepFunImages) {
    return {
      options: DEFAULT_IMAGE_GENERATION_ASPECT_RATIOS,
      allowCustomDimensions: true,
      maxCount: 10,
    };
  }

  if (modelId === 'step-image-edit-2') {
    return {
      options: STEPFUN_IMAGE_EDIT_2_ASPECT_RATIOS,
      allowCustomDimensions: false,
      maxCount: 1,
    };
  }
  if (modelId === 'step-2x-large') {
    return {
      options: STEPFUN_STEP_2X_LARGE_ASPECT_RATIOS,
      allowCustomDimensions: false,
      maxCount: 1,
    };
  }

  // Unknown future StepFun image models fail closed in the UI until their
  // documented size contract is added, while square/automatic generation
  // remains available.
  return {
    options: STEPFUN_UNKNOWN_MODEL_ASPECT_RATIOS,
    allowCustomDimensions: false,
    maxCount: 1,
  };
}

export const IMAGE_GENERATION_QUALITY_OPTIONS: readonly {
  value: ImageGenerationQuality;
  label: string;
}[] = [
  {
    value: 'auto',
    get label() {
      return localizedOptionLabel('creativeStudio.image.quality.auto', '自动');
    },
  },
  {
    value: 'high',
    get label() {
      return localizedOptionLabel('creativeStudio.image.quality.high', '高');
    },
  },
  {
    value: 'medium',
    get label() {
      return localizedOptionLabel('creativeStudio.image.quality.medium', '中');
    },
  },
  {
    value: 'low',
    get label() {
      return localizedOptionLabel('creativeStudio.image.quality.low', '低');
    },
  },
];

export function imageGenerationModelKey(model: ImageGenerationModelIdentity): string {
  return JSON.stringify([model.providerId, model.model]);
}

export function parseImageGenerationModelKey(
  key: string,
  options: readonly ImageGenerationModelOption[]
): ImageGenerationModelIdentity | null {
  const option = options.find((candidate) => imageGenerationModelKey(candidate) === key);
  return option ? { providerId: option.providerId, model: option.model } : null;
}
