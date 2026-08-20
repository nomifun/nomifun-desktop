/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from '../../assets';
import type { CreativeCanvasNode } from '../../domain';
import type { CanvasCasFlushResult } from '../editor';
import type { CreativeNodeAssetPresentation } from '../nodes';
import type { CanvasState } from '../core';

export interface CreativeCanvasProductSelectionCapabilities {
  hasSelection: boolean;
  canGroup: boolean;
  groupIds: string[];
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
  if (!asset || !assetKindMatchesNode(node, asset) || !asset.originalUrl) return null;

  return {
    src:
      node.type === 'image' || node.type === 'panorama'
        ? asset.thumbnailUrl ?? asset.originalUrl
        : asset.originalUrl,
    ...(node.type === 'video' && asset.thumbnailUrl
      ? { posterSrc: asset.thumbnailUrl }
      : {}),
    label: asset.title,
    ...(node.type === 'image' || node.type === 'panorama'
      ? { alt: asset.title }
      : {}),
  };
}
