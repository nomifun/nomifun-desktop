/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * In-house design tokens.
 *
 * Replaces the theme surface of the former third-party platform package.
 * Only the tokens actually consumed by the app are kept; values are copied
 * verbatim from the original library (v0.3.16) so there is zero visual change.
 */

export const Color = {
  PrimaryColor: '#4a58fa',
  FunctionalColor: {
    error: '#f53f3f',
    warn: '#ff7d00',
    success: '#00b42a',
    link: '#165dff',
    yellow: '#fadc19',
    cyan: '#13c1b8',
    purple: '#722ed1',
  },
} as const;

export const Size = {
  IconSize: {
    normal: 16,
  },
} as const;
