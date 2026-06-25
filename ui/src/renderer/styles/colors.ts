/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Theme color configuration based on Figma design tokens
 * This file provides TypeScript types and helper functions for the color system
 *
 * Usage:
 * - CSS: use CSS variables directly: var(--color-bg-0)
 * - UnoCSS: use atomic classes: bg-bg-0, text-text, border-border
 * - TypeScript: use this file for type safety and constants
 */

/**
 * Common icon colors as CSS variable strings for use in fill/stroke props
 */
export const iconColors = {
  primary: 'var(--text-primary)',
  secondary: 'var(--text-secondary)',
  disabled: 'var(--text-disabled)',
  brand: 'var(--brand)',
  danger: 'var(--danger)',
  warning: 'var(--warning)',
  success: 'var(--success)',
} as const;

/**
 * Diff/change colors for file change indicators
 * Used in FileChangesPanel, Markdown diff highlighting, etc.
 */
export const diffColors = {
  /** Green for additions / insertions */
  addition: '#52c41a',
  /** Red for deletions / removals */
  deletion: '#ff4d4f',
  /** Addition background (dark mode) */
  additionBgDark: 'rgba(46,160,67,0.15)',
  /** Addition background (light mode) */
  additionBgLight: '#e6ffec',
  /** Deletion background (dark mode) */
  deletionBgDark: 'rgba(248,81,73,0.15)',
  /** Deletion background (light mode) */
  deletionBgLight: '#ffebe9',
  /** Hunk header background (dark mode) */
  hunkBgDark: 'rgba(56,139,253,0.15)',
  /** Hunk header background (light mode) */
  hunkBgLight: '#ddf4ff',
} as const;
