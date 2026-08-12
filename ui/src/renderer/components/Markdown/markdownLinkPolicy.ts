/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export const IMAGE_MODEL_MANAGEMENT_MARKDOWN_LINK = 'nomifun://model-management/image';
export const IMAGE_MODEL_MANAGEMENT_ROUTE = '/models?section=image';

/**
 * Markdown is model-authored input, so internal navigation is an exact-match
 * allowlist. Variants with a query, fragment, trailing slash, alternate case,
 * or a different nomifun target intentionally remain non-internal.
 */
export const getMarkdownInternalRoute = (href: string): string | undefined =>
  href === IMAGE_MODEL_MANAGEMENT_MARKDOWN_LINK ? IMAGE_MODEL_MANAGEMENT_ROUTE : undefined;
