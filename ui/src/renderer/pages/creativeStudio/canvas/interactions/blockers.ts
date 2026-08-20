/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** Integration gaps intentionally not hidden by compatibility behavior. */
export const CANVAS_INTERACTION_INTEGRATION_BLOCKERS = [
  {
    id: 'editor-render-contract',
    detail: 'CreativeCanvasEditor render contexts do not yet expose resize, connection, context-menu, double-click, or lock callbacks.',
  },
  {
    id: 'blank-connection-node-factory',
    detail: 'Dropping a connection on blank canvas needs the ProductRoute node factory before add-node and connect commands can be dispatched.',
  },
  {
    id: 'system-clipboard-adapter',
    detail: 'System clipboard fallback must read real browser items, upload media through the asset port, and create nodes only from returned assets.',
  },
  {
    id: 'audio-manual-upload',
    detail: 'The current NomiFun manual asset upload contract rejects audio, so canvas audio file drops remain unavailable.',
  },
] as const;
