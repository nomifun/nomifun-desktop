/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { isCreativeAssetDeleted, type CreativeAsset } from '../../assets';
import type {
  CreativeBottomPanelView,
  CreativeCanvasNode,
  CreativeLeftPanelView,
  CreativeRightPanelView,
  CreativeStudioPanelState,
} from '../../domain';
import type { CanvasCasFlushResult, CanvasCasSaveSnapshot } from '../editor';
import type { CreativeNodeAssetPresentation } from '../nodes';
import type { CanvasState } from '../core';
import { creativeStudioProductText } from './i18n';

export interface CreativeCanvasProductSelectionCapabilities {
  hasSelection: boolean;
  canGroup: boolean;
  groupIds: string[];
}

export interface CreativeCanvasProductPanelViews {
  left: CreativeLeftPanelView;
  right: CreativeRightPanelView | null;
  bottom: CreativeBottomPanelView | null;
}

export const CREATIVE_CANVAS_SOURCE_LEFT_PANEL_WIDTH = 280;
export const CREATIVE_CANVAS_SOURCE_AGENT_PANEL_WIDTH = 390;
export const CREATIVE_CANVAS_RIGHT_PANEL_MIN_WIDTH = 320;
export const CREATIVE_CANVAS_RIGHT_PANEL_MAX_WIDTH = 560;

export function clampCreativeCanvasRightPanelWidth(width: number): number {
  return Math.round(
    Math.max(
      CREATIVE_CANVAS_RIGHT_PANEL_MIN_WIDTH,
      Math.min(
        CREATIVE_CANVAS_RIGHT_PANEL_MAX_WIDTH,
        Number.isFinite(width)
          ? width
          : CREATIVE_CANVAS_SOURCE_AGENT_PANEL_WIDTH
      )
    )
  );
}

export function creativeCanvasProductPanelViews(
  panels: CreativeStudioPanelState
): CreativeCanvasProductPanelViews {
  return {
    left: panels.left.activeView,
    right: panels.right.open ? panels.right.activeView : null,
    bottom: panels.bottom.open ? panels.bottom.activeView : null,
  };
}

export function withCreativeCanvasLeftView(
  panels: CreativeStudioPanelState,
  view: CreativeLeftPanelView
): CreativeStudioPanelState {
  return {
    ...panels,
    left: {
      ...panels.left,
      open: true,
      width: CREATIVE_CANVAS_SOURCE_LEFT_PANEL_WIDTH,
      activeView: view,
    },
  };
}

export function withCreativeCanvasLeftPanelOpen(
  panels: CreativeStudioPanelState,
  open: boolean
): CreativeStudioPanelState {
  return {
    ...panels,
    left: {
      ...panels.left,
      open,
    },
  };
}

export function withCreativeCanvasRightView(
  panels: CreativeStudioPanelState,
  view: CreativeRightPanelView | null
): CreativeStudioPanelState {
  return {
    ...panels,
    right: {
      ...panels.right,
      open: view !== null,
      width: clampCreativeCanvasRightPanelWidth(panels.right.width),
      activeView: view ?? panels.right.activeView,
    },
  };
}

export function withCreativeCanvasRightPanelWidth(
  panels: CreativeStudioPanelState,
  width: number
): CreativeStudioPanelState {
  return {
    ...panels,
    right: {
      ...panels.right,
      width: clampCreativeCanvasRightPanelWidth(width),
    },
  };
}

export function withCreativeCanvasBottomView(
  panels: CreativeStudioPanelState,
  view: CreativeBottomPanelView | null
): CreativeStudioPanelState {
  return {
    ...panels,
    bottom: {
      ...panels.bottom,
      open: view !== null,
      activeView: view ?? panels.bottom.activeView,
    },
  };
}

export function creativeCanvasProductSelectionCapabilities(
  state: CanvasState | null
): CreativeCanvasProductSelectionCapabilities {
  if (!state) return { hasSelection: false, canGroup: false, groupIds: [] };
  const byId = new Map(state.document.nodes.map((node) => [node.id, node]));
  const selectedNodes = state.selection.nodeIds
    .map((id) => byId.get(id))
    .filter((node): node is CreativeCanvasNode => Boolean(node));
  const groupIds = selectedNodes
    .filter((node) => node.type === 'group')
    .map((node) => node.id);
  const canGroup =
    selectedNodes.length >= 2 &&
    selectedNodes.every((node) => node.type !== 'group' && node.groupId === null);

  return {
    hasSelection:
      state.selection.nodeIds.length > 0 || state.selection.edgeIds.length > 0,
    canGroup,
    groupIds,
  };
}
export function canLeaveCreativeCanvasAfterFlush(
  result: CanvasCasFlushResult
): boolean {
  return result.status === 'noop' || result.status === 'saved';
}

export const creativeCanvasRevisionConflictMessage = (): string =>
  creativeStudioProductText(
    'creativeStudio.canvas.save.revisionConflict',
    '远端画布已更新，本地更改未覆盖。'
  );

export function creativeCanvasSaveDisplayMessage(
  save: CanvasCasSaveSnapshot
): string | undefined {
  if (save.status === 'conflict') return creativeCanvasRevisionConflictMessage();
  if (save.status === 'error') {
    return (
      save.error?.message ??
      creativeStudioProductText(
        'creativeStudio.canvas.save.failed',
        '画布保存失败。'
      )
    );
  }
  return undefined;
}

export function creativeCanvasBlockedLeaveMessage(
  result: CanvasCasFlushResult
): string | undefined {
  if (result.status === 'conflict') {
    const conflict = creativeCanvasRevisionConflictMessage();
    return creativeStudioProductText(
      'creativeStudio.canvas.save.reloadRequired',
      '{{conflict}}请先重新载入远端版本。',
      { conflict }
    );
  }
  if (result.status === 'error') return result.error.message;
  return undefined;
}

function referencedAssetId(node: CreativeCanvasNode): string | null {
  if (
    node.type === 'image' ||
    node.type === 'panorama' ||
    node.type === 'video' ||
    node.type === 'audio'
  ) {
    return node.data.assetId;
  }
  return null;
}

function assetKindMatchesNode(node: CreativeCanvasNode, asset: CreativeAsset): boolean {
  if (node.type === 'image' || node.type === 'panorama') return asset.kind === 'image';
  if (node.type === 'video') return asset.kind === 'video';
  if (node.type === 'audio') return asset.kind === 'audio';
  return false;
}

/** Resolve only URLs returned by the real asset port; node IDs are never URL templates. */
export function resolveCreativeNodeAssetPresentation(
  node: CreativeCanvasNode,
  assetsById: ReadonlyMap<string, CreativeAsset>
): CreativeNodeAssetPresentation | null {
  const assetId = referencedAssetId(node);
  if (!assetId) return null;
  const asset = assetsById.get(assetId);
  if (asset && isCreativeAssetDeleted(asset)) {
    return { src: '', label: asset.title, deleted: true };
  }
  if (!asset || !assetKindMatchesNode(node, asset) || !asset.originalUrl) return null;

  const posterAsset = node.type === 'video' && node.data.posterAssetId
    ? assetsById.get(node.data.posterAssetId)
    : undefined;
  const posterSrc = node.type === 'video'
    ? posterAsset?.kind === 'image' && !isCreativeAssetDeleted(posterAsset) && posterAsset.originalUrl.trim()
      ? posterAsset.originalUrl
      : asset.thumbnailUrl
    : null;

  return {
    src:
      node.type === 'image' || node.type === 'panorama'
        ? asset.thumbnailUrl ?? asset.originalUrl
        : asset.originalUrl,
    ...(posterSrc
      ? { posterSrc }
      : {}),
    label: asset.title,
    ...(node.type === 'image' || node.type === 'panorama'
      ? { originalSrc: asset.originalUrl, alt: asset.title }
      : {}),
  };
}
