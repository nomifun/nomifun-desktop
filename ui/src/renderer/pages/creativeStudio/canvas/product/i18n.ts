/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { getI18n } from 'react-i18next';

export type CreativeStudioProductTranslationKey = `creativeStudio.${string}`;

type TranslationValues = Record<string, string | number | boolean | null | undefined>;

const interpolateFallback = (
  defaultValue: string,
  values: TranslationValues
): string =>
  defaultValue.replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
    String(values[name] ?? '')
  );

/**
 * Translation entry point for non-React canvas product code whose errors or
 * persisted status text are surfaced by React owners later in the same route.
 */
export function creativeStudioProductText(
  key: CreativeStudioProductTranslationKey,
  defaultValue: string,
  values: TranslationValues = {}
): string {
  const i18n = getI18n();
  if (!i18n) return interpolateFallback(defaultValue, values);
  return i18n.t(key, { ...values, defaultValue });
}
