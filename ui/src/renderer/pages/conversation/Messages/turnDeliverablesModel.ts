/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageToolCall, IMessageToolGroup } from '@/common/chat/chatLib';
import type { MessageId } from '@/common/types/ids';
import type { PersistedToolArtifact } from '@/common/types/platform/toolCallTypes';
import type { TurnFileSummary } from '@/common/types/platform/turnFiles';
import type { TurnDisclosureProcessState } from './turnDisclosureModel';

export interface TurnDeliverableItem {
  identity: string;
  relativePath: string;
  fileName: string;
  absolutePath?: string;
  sizeBytes?: number;
  sha256?: string;
  operation?: 'create' | 'modify' | 'delete';
  artifactId?: PersistedToolArtifact['id'];
  publicationId?: string;
  artifactKind?: PersistedToolArtifact['kind'];
  mimeType?: string;
  /** Only the persisted image receipt grants integrity verification of its locator. */
  tier: 'receipt' | 'change';
  sourceMessageIds: MessageId[];
}

export type VerifiedImageDeliverableItem = TurnDeliverableItem & {
  tier: 'receipt';
  artifactId: PersistedToolArtifact['id'];
  artifactKind: 'image';
  mimeType: string;
};

/** File extensions and model text cannot grant admission to the image gallery. */
export const isVerifiedImageDeliverable = (
  item: TurnDeliverableItem
): item is VerifiedImageDeliverableItem =>
  item.tier === 'receipt' &&
  item.artifactKind === 'image' &&
  typeof item.artifactId === 'string' && item.artifactId.length > 0 &&
  typeof item.mimeType === 'string' && item.mimeType.toLowerCase().startsWith('image/');

export interface TurnGateInfo {
  running: boolean;
  state: TurnDisclosureProcessState;
}

export interface TurnImageCandidate {
  turnId?: MessageId;
  toolMessages: Array<IMessageToolCall | IMessageToolGroup>;
}

const normalizeSlashes = (value: string): string => value.trim().replace(/\\/g, '/').replace(/\/{2,}/g, '/');
const isWindowsAddress = (value: string): boolean => /^[A-Za-z]:[\\/]/.test(value) || /^(?:\\\\|\/\/)/.test(value);
const sameAddress = (left: string, right: string): boolean => {
  if (isWindowsAddress(left) !== isWindowsAddress(right)) return false;
  return isWindowsAddress(left)
    ? normalizeSlashes(left).toLowerCase() === normalizeSlashes(right).toLowerCase()
    : left === right;
};

/** Only receipt display paths are normalized; native proof locators remain untouched. */
const receiptDisplayPath = (artifact: PersistedToolArtifact, workspaceRoots: string[]): string => {
  const path = normalizeSlashes(artifact.relative_path || artifact.path);
  if (!(path.startsWith('/') || /^[A-Za-z]:\//.test(path))) return path.replace(/^\.\//, '');
  const windows = isWindowsAddress(artifact.path);
  for (const rawRoot of workspaceRoots) {
    if (isWindowsAddress(rawRoot) !== windows) continue;
    const root = normalizeSlashes(rawRoot).replace(/\/+$/, '');
    const sameRoot = windows ? path.toLowerCase().startsWith(`${root.toLowerCase()}/`) : path.startsWith(`${root}/`);
    if (sameRoot) return path.slice(root.length + 1);
  }
  return path.split('/').at(-1) || path;
};

/**
 * Verified image receipts retain their dedicated UI contract. Ordinary files
 * have exactly one source: the canonical Turn file query, never tool names,
 * arguments, output text, or the currently loaded transcript window.
 */
export function collectVerifiedTurnImages(
  candidates: TurnImageCandidate[],
  turnGates: ReadonlyMap<string, TurnGateInfo>,
  workspaceRoots: string[] = []
): Map<MessageId, VerifiedImageDeliverableItem[]> {
  const byTurn = new Map<MessageId, Map<string, VerifiedImageDeliverableItem>>();
  for (const { turnId, toolMessages } of candidates) {
    if (!turnId || !turnGates.has(turnId) || turnGates.get(turnId)?.running) continue;
    for (const message of toolMessages) {
      if (message.type !== 'tool_call' || message.content.status !== 'completed') continue;
      for (const artifact of message.content.artifacts ?? []) {
        const relativePath = receiptDisplayPath(artifact, workspaceRoots);
        const item: TurnDeliverableItem = {
          identity: `artifact:${artifact.id}`,
          relativePath,
          fileName: relativePath.split('/').at(-1) || relativePath,
          absolutePath: artifact.path,
          sizeBytes: artifact.size_bytes,
          sha256: artifact.sha256,
          artifactId: artifact.id,
          artifactKind: artifact.kind,
          mimeType: artifact.mime_type,
          tier: 'receipt',
          sourceMessageIds: [message.message_id ?? message.msg_id].filter((id): id is MessageId => Boolean(id)),
        };
        if (!isVerifiedImageDeliverable(item)) continue;
        const bucket = byTurn.get(turnId) ?? new Map<string, VerifiedImageDeliverableItem>();
        if (!bucket.has(artifact.id)) bucket.set(artifact.id, item);
        byTurn.set(turnId, bucket);
      }
    }
  }
  return new Map([...byTurn].map(([id, items]) => [id, [...items.values()]]));
}

/** Source locators preserve filenames and preview associations; publication IDs stay separate. */
export function buildTurnFileItems(
  summary: TurnFileSummary | undefined,
  verifiedImages: VerifiedImageDeliverableItem[] = []
): TurnDeliverableItem[] {
  const files: TurnDeliverableItem[] = (summary?.files ?? [])
    .filter((file) => file.operation === 'delete' || !(
      file.path && file.sha256 && verifiedImages.some((image) =>
        image.absolutePath && image.sha256 === file.sha256 && sameAddress(image.absolutePath, file.path!)
      )
    ))
    .map((file) => ({
      identity: file.identity,
      relativePath: file.relative_path,
      fileName: file.file_name,
      absolutePath: file.path,
      operation: file.operation,
      sizeBytes: file.size_bytes,
      sha256: file.sha256,
      publicationId: file.published?.artifact_id,
      mimeType: file.published?.mime_type,
      tier: 'change',
      sourceMessageIds: [],
    }));
  return [...verifiedImages, ...files];
}
