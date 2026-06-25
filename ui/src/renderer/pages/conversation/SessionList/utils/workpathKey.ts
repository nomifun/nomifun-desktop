/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

/** 派生 workpath 的唯一 key。defaultpath（Nomi 默认工作路径）用哨兵常量。 */
export const DEFAULT_WORKPATH_KEY = '__default__';

export function workpathKey(path: string | undefined | null): string {
  const trimmed = (path ?? '').trim();
  if (!trimmed) return DEFAULT_WORKPATH_KEY;
  const slashed = trimmed.replace(/\\/g, '/');
  if (slashed === '/') return '/';
  return slashed.replace(/\/+$/, '');
}
