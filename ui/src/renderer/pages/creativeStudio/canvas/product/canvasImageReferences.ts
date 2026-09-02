/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeAsset } from '../../assets';
import type {
  CreativeCanvasConnection,
  CreativeCanvasNodeKind,
} from '../../domain';
import type { CanvasState } from '../core';

export type CanvasImageReferenceNodeKind = Extract<
  CreativeCanvasNodeKind,
  'image' | 'panorama'
>;

/** Product-level decoded input budget; the backend enforces the same ceiling. */
export const MAX_CANVAS_IMAGE_REFERENCE_BYTES = 256 * 1024 * 1024;

type CanvasImageReferenceNode = Extract<
  CanvasState['document']['nodes'][number],
  { type: CanvasImageReferenceNodeKind }
>;

/**
 * One provider input derived from the active node's base image or a direct
 * inbound canvas connection.
 *
 * `ordinal` is intentionally derived from the durable connection array rather
 * than node position. It is the one-based number used by the provider prompt.
 */
export interface CanvasImageReference {
  ordinal: number;
  providerLabel: string;
  /** Null only for the active image node's own pinned base image. */
  connection: CreativeCanvasConnection | null;
  sourceNodeId: string;
  sourceNodeKind: CanvasImageReferenceNodeKind;
  assetId: string;
  asset: CreativeAsset;
  displayName: string;
}

export interface CanvasTextReference {
  sourceNodeId: string;
  connection: CreativeCanvasConnection;
  ordinal: number;
  text: string;
}

export type CanvasImageReferenceIssue =
  | { code: 'source_text_empty'; connectionId: string; sourceNodeId: string }
  | {
      code: 'target_node_missing';
      targetNodeId: string;
    }
  | {
      code: 'target_node_kind_unsupported';
      targetNodeId: string;
      targetNodeKind: CreativeCanvasNodeKind;
    }
  | {
      code: 'target_asset_unresolved';
      targetNodeId: string;
      assetId: string;
    }
  | {
      code: 'target_asset_kind_unsupported';
      targetNodeId: string;
      assetId: string;
      assetKind: CreativeAsset['kind'];
    }
  | {
      code: 'source_node_missing';
      connectionId: string;
      sourceNodeId: string;
    }
  | {
      code: 'source_node_kind_unsupported';
      connectionId: string;
      sourceNodeId: string;
      sourceNodeKind: CreativeCanvasNodeKind;
    }
  | {
      code: 'source_asset_id_missing';
      connectionId: string;
      sourceNodeId: string;
      sourceNodeKind: CanvasImageReferenceNodeKind;
    }
  | {
      code: 'source_asset_unresolved';
      connectionId: string;
      sourceNodeId: string;
      assetId: string;
    }
  | {
      code: 'source_asset_kind_unsupported';
      connectionId: string;
      sourceNodeId: string;
      assetId: string;
      assetKind: CreativeAsset['kind'];
    }
  | {
      code: 'duplicate_asset';
      connectionId: string;
      sourceNodeId: string;
      assetId: string;
      firstConnectionId: string | null;
      firstSourceNodeId: string;
    };

export interface CanvasImageReferenceResolution {
  targetNodeId: string;
  /** Number of direct inbound connections, including invalid ones. */
  inboundConnectionCount: number;
  references: CanvasImageReference[];
  textReferences: CanvasTextReference[];
  issues: CanvasImageReferenceIssue[];
}

const isReferenceNode = (
  node: CanvasState['document']['nodes'][number]
): node is CanvasImageReferenceNode => node.type === 'image' || node.type === 'panorama';

const sourceAssetId = (node: CanvasImageReferenceNode): string | null => {
  const value = node.data.assetId;
  return typeof value === 'string' && value.trim() ? value : null;
};

/** Asset ids needed to hydrate the active node's base plus direct media inputs. */
export function canvasImageReferenceAssetIds(
  state: Pick<CanvasState, 'document'>,
  targetNodeId: string
): string[] {
  const nodesById = new Map(state.document.nodes.map((node) => [node.id, node]));
  const target = nodesById.get(targetNodeId);
  if (!target || target.type !== 'image') return [];
  const ids: string[] = [];
  const ownAssetId = sourceAssetId(target);
  if (ownAssetId) ids.push(ownAssetId);
  for (const connection of state.document.connections) {
    if (connection.targetNodeId !== targetNodeId) continue;
    const source = nodesById.get(connection.sourceNodeId);
    if (!source || !isReferenceNode(source)) continue;
    const assetId = sourceAssetId(source);
    if (assetId) ids.push(assetId);
  }
  return [...new Set(ids)];
}

/**
 * Resolve the target image node's direct inbound references without creating
 * attachment state outside the canonical canvas graph.
 *
 * The active node's own image is pinned first; valid inbound references then
 * retain `document.connections` order. Invalid connections
 * do not consume a provider ordinal. The first connection to a repeated asset
 * retains its place; later duplicates are rejected so callers can never send
 * the same asset twice by accident.
 */
export function resolveCanvasImageReferences(
  state: Pick<CanvasState, 'document'>,
  targetNodeId: string,
  assets: readonly CreativeAsset[]
): CanvasImageReferenceResolution {
  const target = state.document.nodes.find((node) => node.id === targetNodeId);
  if (!target) {
    return {
      targetNodeId,
      inboundConnectionCount: 0,
      references: [],
      textReferences: [],
      issues: [{ code: 'target_node_missing', targetNodeId }],
    };
  }
  if (target.type !== 'image') {
    return {
      targetNodeId,
      inboundConnectionCount: 0,
      references: [],
      textReferences: [],
      issues: [
        {
          code: 'target_node_kind_unsupported',
          targetNodeId,
          targetNodeKind: target.type,
        },
      ],
    };
  }

  const nodesById = new Map(state.document.nodes.map((node) => [node.id, node]));
  const assetsById = new Map(assets.map((asset) => [asset.id, asset]));
  const inboundConnections = state.document.connections.filter(
    (connection) => connection.targetNodeId === targetNodeId
  );
  const references: CanvasImageReference[] = [];
  const textReferences: CanvasTextReference[] = [];
  const issues: CanvasImageReferenceIssue[] = [];
  const firstReferenceByAssetId = new Map<string, CanvasImageReference>();

  const targetAssetId = sourceAssetId(target);
  if (targetAssetId) {
    const asset = assetsById.get(targetAssetId);
    if (!asset) {
      issues.push({
        code: 'target_asset_unresolved',
        targetNodeId,
        assetId: targetAssetId,
      });
    } else if (asset.kind !== 'image') {
      issues.push({
        code: 'target_asset_kind_unsupported',
        targetNodeId,
        assetId: targetAssetId,
        assetKind: asset.kind,
      });
    } else {
      const reference: CanvasImageReference = {
        ordinal: 1,
        providerLabel: 'Reference 1',
        connection: null,
        sourceNodeId: target.id,
        sourceNodeKind: 'image',
        assetId: targetAssetId,
        asset,
        displayName: asset.title.trim() || 'Current image',
      };
      references.push(reference);
      firstReferenceByAssetId.set(targetAssetId, reference);
    }
  }

  for (const connection of inboundConnections) {
    const source = nodesById.get(connection.sourceNodeId);
    if (!source) {
      issues.push({
        code: 'source_node_missing',
        connectionId: connection.id,
        sourceNodeId: connection.sourceNodeId,
      });
      continue;
    }
    if (source.type === 'text') {
      const text = source.data.text.trim();
      textReferences.push({ sourceNodeId: source.id, connection, ordinal: textReferences.length + 1, text });
      if (!text) issues.push({ code: 'source_text_empty', connectionId: connection.id, sourceNodeId: source.id });
      continue;
    }
    if (!isReferenceNode(source)) {
      // Ignore only a proven config -> result-image lineage edge. A manually
      // connected config targeting an unrelated/empty image remains invalid.
      if (
        source.type === 'config' &&
        targetAssetId !== null &&
        source.data.resultAssetIds.includes(targetAssetId)
      ) {
        continue;
      }
      issues.push({
        code: 'source_node_kind_unsupported',
        connectionId: connection.id,
        sourceNodeId: source.id,
        sourceNodeKind: source.type,
      });
      continue;
    }

    const assetId = sourceAssetId(source);
    if (!assetId) {
      issues.push({
        code: 'source_asset_id_missing',
        connectionId: connection.id,
        sourceNodeId: source.id,
        sourceNodeKind: source.type,
      });
      continue;
    }

    const asset = assetsById.get(assetId);
    if (!asset) {
      issues.push({
        code: 'source_asset_unresolved',
        connectionId: connection.id,
        sourceNodeId: source.id,
        assetId,
      });
      continue;
    }
    if (asset.kind !== 'image') {
      issues.push({
        code: 'source_asset_kind_unsupported',
        connectionId: connection.id,
        sourceNodeId: source.id,
        assetId,
        assetKind: asset.kind,
      });
      continue;
    }

    const firstReference = firstReferenceByAssetId.get(assetId);
    if (firstReference) {
      issues.push({
        code: 'duplicate_asset',
        connectionId: connection.id,
        sourceNodeId: source.id,
        assetId,
        firstConnectionId: firstReference.connection?.id ?? null,
        firstSourceNodeId: firstReference.sourceNodeId,
      });
      continue;
    }

    const ordinal = references.length + 1;
    const reference: CanvasImageReference = {
      ordinal,
      providerLabel: `Reference ${ordinal}`,
      connection,
      sourceNodeId: source.id,
      sourceNodeKind: source.type,
      assetId,
      asset,
      displayName: asset.title.trim() || `Reference ${ordinal}`,
    };
    references.push(reference);
    firstReferenceByAssetId.set(assetId, reference);
  }

  return {
    targetNodeId,
    inboundConnectionCount: inboundConnections.length,
    references,
    textReferences,
    issues,
  };
}

/**
 * Structured identity carried by an editor mention chip. Offsets are UTF-16
 * string offsets, matching JavaScript selection APIs. `tokenText` protects
 * against applying stale offsets to an edited prompt.
 */
export interface AuthoredCanvasImagePromptMention {
  sourceNodeId: string;
  start: number;
  end: number;
  tokenText: string;
}

export type CanvasImagePromptCompilationIssue =
  | {
      code: 'mention_range_invalid';
      mentionIndex: number;
      start: number;
      end: number;
    }
  | {
      code: 'mention_token_mismatch';
      mentionIndex: number;
      start: number;
      end: number;
      expected: string;
      actual: string;
    }
  | {
      code: 'mention_ranges_overlap';
      mentionIndex: number;
      previousMentionIndex: number;
    }
  | {
      code: 'mention_reference_disconnected';
      mentionIndex: number;
      sourceNodeId: string;
    };

export type CanvasImagePromptCompilation =
  | {
      ok: true;
      authoredPrompt: string;
      providerPrompt: string;
      referencedSourceNodeIds: string[];
      issues: [];
    }
  | {
      ok: false;
      authoredPrompt: string;
      providerPrompt: null;
      referencedSourceNodeIds: string[];
      issues: CanvasImagePromptCompilationIssue[];
    };

interface IndexedMention {
  mention: AuthoredCanvasImagePromptMention;
  mentionIndex: number;
}

/** Resolve image mentions and connected text into the provider prompt. */
export function compileCanvasImageReferencePrompt(
  authoredPrompt: string,
  mentions: readonly AuthoredCanvasImagePromptMention[],
  references: readonly CanvasImageReference[],
  textReferences: readonly CanvasTextReference[] = []
): CanvasImagePromptCompilation {
  const referencesBySourceNodeId = new Map(
    [
      ...references,
      ...textReferences.map((reference) => ({
        sourceNodeId: reference.sourceNodeId,
        providerLabel: reference.text,
      })),
    ].map((reference) => [reference.sourceNodeId, reference])
  );
  const sorted: IndexedMention[] = mentions
    .map((mention, mentionIndex) => ({ mention, mentionIndex }))
    .sort(
      (left, right) =>
        left.mention.start - right.mention.start ||
        left.mention.end - right.mention.end ||
        left.mentionIndex - right.mentionIndex
    );
  const issues: CanvasImagePromptCompilationIssue[] = [];
  const valid: Array<IndexedMention & { reference: { providerLabel: string } }> = [];
  let previous: IndexedMention | null = null;

  for (const indexed of sorted) {
    const { mention, mentionIndex } = indexed;
    if (
      !Number.isInteger(mention.start) ||
      !Number.isInteger(mention.end) ||
      mention.start < 0 ||
      mention.end <= mention.start ||
      mention.end > authoredPrompt.length
    ) {
      issues.push({
        code: 'mention_range_invalid',
        mentionIndex,
        start: mention.start,
        end: mention.end,
      });
      continue;
    }
    if (previous && mention.start < previous.mention.end) {
      issues.push({
        code: 'mention_ranges_overlap',
        mentionIndex,
        previousMentionIndex: previous.mentionIndex,
      });
      if (mention.end > previous.mention.end) previous = indexed;
      continue;
    }
    previous = indexed;

    const actual = authoredPrompt.slice(mention.start, mention.end);
    if (actual !== mention.tokenText) {
      issues.push({
        code: 'mention_token_mismatch',
        mentionIndex,
        start: mention.start,
        end: mention.end,
        expected: mention.tokenText,
        actual,
      });
      continue;
    }
    const reference = referencesBySourceNodeId.get(mention.sourceNodeId);
    if (!reference) {
      issues.push({
        code: 'mention_reference_disconnected',
        mentionIndex,
        sourceNodeId: mention.sourceNodeId,
      });
      continue;
    }
    valid.push({ ...indexed, reference });
  }

  const referencedSourceNodeIds = [
    ...new Set(valid.map(({ mention }) => mention.sourceNodeId)),
  ];
  if (issues.length > 0) {
    return {
      ok: false,
      authoredPrompt,
      providerPrompt: null,
      referencedSourceNodeIds,
      issues,
    };
  }

  let cursor = 0;
  let providerPrompt = '';
  for (const { mention, reference } of valid) {
    providerPrompt += authoredPrompt.slice(cursor, mention.start);
    providerPrompt += reference.providerLabel;
    cursor = mention.end;
  }
  providerPrompt += authoredPrompt.slice(cursor);

  // Connected text is prompt input, not an image attachment. Explicit mentions
  // place it inline; otherwise include it once before the authored instruction.
  const mentionedNodeIds = new Set(referencedSourceNodeIds);
  const unmentionedText = textReferences.filter(
    (reference) => !mentionedNodeIds.has(reference.sourceNodeId) && reference.text
  );
  if (unmentionedText.length > 0) {
    providerPrompt = [...unmentionedText.map((reference) => reference.text), providerPrompt]
      .filter(Boolean).join('\n\n');
    referencedSourceNodeIds.push(...unmentionedText.map((reference) => reference.sourceNodeId));
  }

  return {
    ok: true,
    authoredPrompt,
    providerPrompt,
    referencedSourceNodeIds,
    issues: [],
  };
}

export type CanvasImageGenerationBlocker =
  | {
      code: 'reference_resolution_failed';
      issue: CanvasImageReferenceIssue;
    }
  | {
      code: 'prompt_compilation_failed';
      issue: CanvasImagePromptCompilationIssue;
    }
  | {
      code: 'reference_limit_unknown';
      referenceCount: number;
    }
  | {
      code: 'reference_limit_exceeded';
      referenceCount: number;
      maxInputImages: number;
    }
  | {
      code: 'reference_bytes_exceeded';
      totalBytes: number;
      maxInputBytes: number;
    };

export interface CanvasImageGenerationGate {
  allowed: boolean;
  operation: 't2i' | 'i2i';
  referenceCount: number;
  blockers: CanvasImageGenerationBlocker[];
}

/**
 * Fail closed for graph/prompt errors and for multi-image requests whose model
 * limit is unknown. A null limit means capability metadata is unavailable.
 */
export function evaluateCanvasImageGenerationGate(input: {
  resolution: CanvasImageReferenceResolution;
  compilation: CanvasImagePromptCompilation;
  maxInputImages: number | null;
  /** Exact protocol declares multi-image transport but no stable numeric cap. */
  allowMultipleWithoutKnownMaximum?: boolean;
}): CanvasImageGenerationGate {
  const referenceCount = input.resolution.references.length;
  const blockers: CanvasImageGenerationBlocker[] = [
    ...input.resolution.issues.map(
      (issue): CanvasImageGenerationBlocker => ({
        code: 'reference_resolution_failed',
        issue,
      })
    ),
    ...input.compilation.issues.map(
      (issue): CanvasImageGenerationBlocker => ({
        code: 'prompt_compilation_failed',
        issue,
      })
    ),
  ];
  const normalizedLimit =
    input.maxInputImages !== null &&
    Number.isInteger(input.maxInputImages) &&
    input.maxInputImages >= 0
      ? input.maxInputImages
      : null;

  if (
    normalizedLimit === null &&
    referenceCount > 1 &&
    !input.allowMultipleWithoutKnownMaximum
  ) {
    blockers.push({ code: 'reference_limit_unknown', referenceCount });
  } else if (normalizedLimit !== null && referenceCount > normalizedLimit) {
    blockers.push({
      code: 'reference_limit_exceeded',
      referenceCount,
      maxInputImages: normalizedLimit,
    });
  }
  const knownByteSizes = input.resolution.references.map(
    (reference) => reference.asset.bytes
  );
  if (knownByteSizes.every((bytes): bytes is number => bytes !== null)) {
    const totalBytes = knownByteSizes.reduce((total, bytes) => total + bytes, 0);
    if (totalBytes > MAX_CANVAS_IMAGE_REFERENCE_BYTES) {
      blockers.push({
        code: 'reference_bytes_exceeded',
        totalBytes,
        maxInputBytes: MAX_CANVAS_IMAGE_REFERENCE_BYTES,
      });
    }
  }

  return {
    allowed: blockers.length === 0,
    operation: referenceCount > 0 ? 'i2i' : 't2i',
    referenceCount,
    blockers,
  };
}
