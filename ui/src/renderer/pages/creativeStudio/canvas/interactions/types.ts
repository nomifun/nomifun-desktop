/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CanvasCommand, CanvasConnectionErrorCode, CanvasPoint } from '../core';
import type { CreativeAssetUploadRejection } from '../../assets/page/model';

export type CanvasContextTarget =
  | { kind: 'canvas' }
  | { kind: 'node'; nodeId: string }
  | { kind: 'edge'; edgeId: string };

export type CanvasNodeOpenMode =
  | 'edit-text'
  | 'compose'
  | 'preview-media'
  | 'open-director'
  | 'inspect-group';

export type CanvasDropImportKind = 'image' | 'video';

/**
 * Effects which belong to the product shell rather than the canonical canvas
 * reducer. Consumers must resolve them through real UI and asset boundaries.
 */
export type CanvasIntegrationIntent =
  | {
      type: 'system-clipboard/read';
      /** The product chooses how a real clipboard item becomes an asset/node. */
      worldPosition?: CanvasPoint;
    }
  | { type: 'transient-ui/dismiss' }
  | {
      type: 'context-menu/open';
      target: CanvasContextTarget;
      clientPosition: CanvasPoint;
    }
  | {
      type: 'node/open';
      nodeId: string;
      mode: CanvasNodeOpenMode;
    }
  | {
      type: 'canvas/create-node-menu/open';
      worldPosition: CanvasPoint;
    }
  | {
      type: 'connection/create-node-menu/open';
      fixedNodeId: string;
      fixedHandle: 'source' | 'target';
      fixedHandleId: string | null;
      fixedNodeIds?: readonly string[];
      worldPosition: CanvasPoint;
    }
  | {
      type: 'connection/rejected';
      code: CanvasConnectionErrorCode | 'no_valid_drop_target';
    }
  | {
      type: 'connection/batch-created';
      count: number;
      skippedCount: number;
    }
  | {
      type: 'connection/created';
      sourceNodeId: string;
      targetNodeId: string;
    }
  | {
      type: 'asset/import-file';
      /** This is the real browser File; the controller never invents an asset. */
      file: File;
      kind: CanvasDropImportKind;
      worldPosition: CanvasPoint;
      /** Source parity: a real 2:1 image asks after upload metadata is known. */
      panoramaChoice: 'after-upload-if-2-to-1' | 'not-applicable';
    }
  | {
      type: 'asset/import-feedback';
      rejected: Array<{ fileName: string; reason: CreativeAssetUploadRejection }>;
      ignoredAcceptedFileNames: string[];
    };

export interface CanvasInteractionResolution {
  handled: boolean;
  preventDefault: boolean;
  commands: CanvasCommand[];
  intents: CanvasIntegrationIntent[];
}

export const unhandledCanvasInteraction = (): CanvasInteractionResolution => ({
  handled: false,
  preventDefault: false,
  commands: [],
  intents: [],
});

export function canvasInteractionResolution(
  input: Partial<CanvasInteractionResolution> = {}
): CanvasInteractionResolution {
  return {
    handled: input.handled ?? true,
    preventDefault: input.preventDefault ?? true,
    commands: input.commands ?? [],
    intents: input.intents ?? [],
  };
}
