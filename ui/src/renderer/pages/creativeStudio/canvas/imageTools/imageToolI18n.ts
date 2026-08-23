/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import i18n from "i18next";

type ImageToolInterpolation = Record<string, string | number>;

/**
 * Non-React image operations share the renderer's i18next instance so errors
 * and derived asset names use the language active when the operation runs.
 */
export function translateCreativeImageTool(
  key: string,
  values?: ImageToolInterpolation,
): string {
  const translated = i18n.t(key, values);
  return typeof translated === "string" ? translated : key;
}
