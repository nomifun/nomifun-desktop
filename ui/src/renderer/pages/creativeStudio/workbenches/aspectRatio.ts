/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

const EXPLICIT_RATIO_PATTERN = /(^|[^\d])(\d{1,4})\s*[:：xX×]\s*(\d{1,4})(?!\d)/g;

const ratioParts = (value: string): readonly [number, number] | null => {
  const match = value.trim().match(/^(\d{1,4})\s*[:：]\s*(\d{1,4})$/);
  if (!match) return null;
  const width = Number(match[1]);
  const height = Number(match[2]);
  return width > 0 && height > 0 ? [width, height] : null;
};

const equivalentRatio = (
  left: readonly [number, number],
  right: readonly [number, number]
): boolean => left[0] * right[1] === right[0] * left[1];

const availableRatios = (supportedRatios: readonly string[]): string[] =>
  [...new Set(supportedRatios)].filter((ratio) => ratio !== 'auto' && ratioParts(ratio));

const preferredRatio = (
  supportedRatios: readonly string[],
  preferences: readonly string[]
): string | null => {
  for (const preference of preferences) {
    const preferredParts = ratioParts(preference);
    if (!preferredParts) continue;
    const match = supportedRatios.find((ratio) => {
      const supportedParts = ratioParts(ratio);
      return supportedParts !== null && equivalentRatio(preferredParts, supportedParts);
    });
    if (match) return match;
  }
  return null;
};

/**
 * Extract a provider-supported ratio from a media prompt. Explicit numeric
 * ratios always win over broader orientation language. Unsupported numeric
 * ratios are not silently replaced with a different ratio.
 */
export function inferPromptAspectRatio(
  prompt: string,
  supportedRatios: readonly string[]
): string | null {
  const available = availableRatios(supportedRatios);

  EXPLICIT_RATIO_PATTERN.lastIndex = 0;
  for (const match of prompt.matchAll(EXPLICIT_RATIO_PATTERN)) {
    const requested: readonly [number, number] = [Number(match[2]), Number(match[3])];
    if (requested[0] <= 0 || requested[1] <= 0) continue;
    const supported = available.find((ratio) => {
      const parts = ratioParts(ratio);
      return parts !== null && equivalentRatio(requested, parts);
    });
    if (supported) return supported;

    // A numeric ratio is more specific than any direction word beside it. If
    // the selected provider cannot represent it, preserve model-side auto.
    return null;
  }

  if (/(?:正方形|方形(?:构图|画面|图片|图)?|square)/i.test(prompt)) {
    return preferredRatio(available, ['1:1']);
  }
  if (/(?:竖(?:构图|版|屏|图|向|幅)|纵向|portrait|vertical)/i.test(prompt)) {
    return preferredRatio(available, ['9:16', '3:4', '2:3']);
  }
  if (/(?:横(?:构图|版|屏|图|向|幅)|宽幅|landscape|horizontal|widescreen)/i.test(prompt)) {
    return preferredRatio(available, ['16:9', '4:3', '3:2', '21:9']);
  }
  return null;
}

/** Apply the workbench priority contract without mutating the saved UI value. */
export function resolveMediaAspectRatio(
  configuredAspectRatio: string,
  prompt: string,
  supportedRatios: readonly string[]
): string | null {
  return configuredAspectRatio === 'auto'
    ? inferPromptAspectRatio(prompt, supportedRatios)
    : configuredAspectRatio;
}
