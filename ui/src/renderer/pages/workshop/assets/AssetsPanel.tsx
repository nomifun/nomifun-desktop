/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Asset library panel (M4 module).
 *
 * The canvas page mounts this panel and toggles it with the `A` shortcut.
 * Props below are the frozen M0 contract between the canvas (M1) and the
 * asset library (M4); M4 replaces this stub with the full implementation.
 */

import type { ReactElement } from 'react';

import type { WorkshopAsset } from '../types';

export interface AssetsPanelProps {
  /** Canvas the panel is opened from (used to scope "insert" actions). */
  canvasId: string;
  open: boolean;
  onClose: () => void;
  /** Called when the user picks "insert into canvas" on an asset. */
  onInsertAsset: (asset: WorkshopAsset) => void;
}

export default function AssetsPanel(_props: AssetsPanelProps): ReactElement | null {
  return null;
}
