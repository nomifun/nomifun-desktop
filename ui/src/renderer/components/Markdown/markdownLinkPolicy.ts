/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export const IMAGE_MODEL_MANAGEMENT_MARKDOWN_LINK = 'nomifun://model-management/image';
export const IMAGE_MODEL_MANAGEMENT_ROUTE = '/models?section=image';
const MODEL_MANAGEMENT_ROUTES: Readonly<Record<string, string>> = {
  'nomifun://model-management/models': '/models?section=models',
  'nomifun://model-management/chat': '/models?section=chat',
  [IMAGE_MODEL_MANAGEMENT_MARKDOWN_LINK]: IMAGE_MODEL_MANAGEMENT_ROUTE,
  'nomifun://model-management/image-edit': '/models?section=image-edit',
  'nomifun://model-management/video': '/models?section=video',
  'nomifun://model-management/music': '/models?section=music',
  'nomifun://model-management/tts': '/models?section=tts',
};

/**
 * Markdown is model-authored input, so internal navigation is an exact-match
 * allowlist. Variants with a query, fragment, trailing slash, alternate case,
 * or a different nomifun target intentionally remain non-internal.
 */
export const getMarkdownInternalRoute = (href: string): string | undefined =>
  MODEL_MANAGEMENT_ROUTES[href];
