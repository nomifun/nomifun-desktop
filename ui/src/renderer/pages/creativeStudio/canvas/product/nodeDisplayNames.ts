/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import type { CreativeCanvasNode } from '../../domain';
import type { CreativeAsset } from '../../assets';
import type { TFunction } from 'i18next';

const KIND_LABELS = {
  text: 'creativeStudio.canvas.nodeKinds.text',
  image: 'creativeStudio.canvas.nodeKinds.image',
  video: 'creativeStudio.canvas.nodeKinds.video',
  audio: 'creativeStudio.canvas.nodeKinds.audio',
  timeline: 'creativeStudio.canvas.nodeKinds.timeline',
  panorama: 'creativeStudio.canvas.nodeKinds.panorama',
  group: 'creativeStudio.canvas.nodeKinds.group',
} as const;

/** Document order, rather than selection or z-order, determines fallback names. */
export function canvasNodeDisplayNames(
  nodes: readonly CreativeCanvasNode[],
  assets: ReadonlyMap<string, CreativeAsset>,
  t: TFunction
): Map<string, string> {
  const counts = new Map<string, number>();
  const names = new Map<string, string>();
  for (const node of nodes) {
    if (node.type === 'config') continue;
    const ordinal = (counts.get(node.type) ?? 0) + 1;
    counts.set(node.type, ordinal);
    const asset = 'assetId' in node.data && node.data.assetId
      ? assets.get(node.data.assetId) : undefined;
    const localName = node.type === 'image' ? node.data.caption
      : node.type === 'audio' || node.type === 'timeline' || node.type === 'group'
        ? node.data.title
        : '';
    names.set(node.id, asset?.title.trim() || localName.trim() || `${t(KIND_LABELS[node.type])}${ordinal}`);
  }
  return names;
}
