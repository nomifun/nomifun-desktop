/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CSSProperties } from 'react';

/**
 * Viewport-portaled canvas composers and node toolbars use 1600 and 1601.
 * Canvas modals must be mounted at the document root and stay above both.
 */
export const CREATIVE_CANVAS_MODAL_Z_INDEX = 1700;

export const CREATIVE_CANVAS_MODAL_LAYER_STYLE: CSSProperties = {
  zIndex: CREATIVE_CANVAS_MODAL_Z_INDEX,
};

export const getCreativeCanvasModalPopupContainer = (): HTMLElement => document.body;
