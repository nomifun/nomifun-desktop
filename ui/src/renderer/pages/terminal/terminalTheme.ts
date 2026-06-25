/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ITheme } from '@xterm/xterm';

/**
 * Constant-dark terminal theme. The terminal canvas is always dark — matching
 * VS Code / iTerm / Warp — regardless of the app's light/dark mode; the
 * surrounding card (header/border/background) follows the app theme via semantic
 * tokens (see `--terminal-surface-bg` / `--terminal-border`). Keeping the canvas
 * dark avoids the light-mode "dark box in a near-white page" clash and gives
 * TUIs (claude, vim) a stable, legible palette.
 *
 * Palette: One Dark–class, tuned for contrast on #1b1d23.
 */
export const TERMINAL_THEME: ITheme = {
  background: '#1b1d23',
  foreground: '#d7dae0',
  cursor: '#d7dae0',
  cursorAccent: '#1b1d23',
  selectionBackground: 'rgba(122,131,178,0.40)',
  selectionForeground: '#ffffff',
  black: '#3f4451',
  red: '#e06c75',
  green: '#98c379',
  yellow: '#e5c07b',
  blue: '#61afef',
  magenta: '#c678dd',
  cyan: '#56b6c2',
  white: '#d7dae0',
  brightBlack: '#4f5666',
  brightRed: '#ff7b86',
  brightGreen: '#a8d98a',
  brightYellow: '#f0cd8b',
  brightBlue: '#74bbff',
  brightMagenta: '#d68ee8',
  brightCyan: '#66c6d2',
  brightWhite: '#ffffff',
};

/** Typography options applied to the xterm Terminal constructor. */
export const TERMINAL_TYPOGRAPHY = {
  fontSize: 13,
  lineHeight: 1.25,
  letterSpacing: 0,
  fontWeight: 400 as const,
  fontWeightBold: 600 as const,
};
