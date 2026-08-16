/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  IMessageToolCall,
  IMessageToolGroup,
} from '@/common/chat/chatLib';
import type { PersistedToolArtifact } from '@/common/types/platform/toolCallTypes';
import type { MessageId } from '@/common/types/ids';
import { parseDiff, type FileChangeInfo } from '@/renderer/utils/file/diffUtils';
import { isSuccessfulWriteFileResult } from './components/toolGroupArtifactVisibility';
import type { TurnDisclosureProcessState, TurnDisclosureRole } from './turnDisclosureModel';
import type { WriteFileResult } from './types';

export type TurnDeliverableTier = 'receipt' | 'reported';

export type TurnDeliverableCarrier =
  | 'tool_call_artifact'
  | 'tool_call_args'
  | 'tool_group_write_file'
  | 'write_file_diff';

export interface TurnDeliverableSource {
  carrier: TurnDeliverableCarrier;
  callId?: string;
  sourceMessageIds: MessageId[];
}

export interface TurnDeliverableItem {
  /** '/'-separated display path; workspace-relative whenever resolvable. */
  relativePath: string;
  fileName: string;
  /** Normalized absolute path when the carrier provided one. */
  absolutePath?: string;
  sizeBytes?: number;
  sha256?: string;
  /** Present only when this item came from a committed artifact receipt. */
  artifactId?: PersistedToolArtifact['id'];
  /** Backend-verified artifact kind; never inferred from a filename or Markdown URL. */
  artifactKind?: PersistedToolArtifact['kind'];
  /** Backend-verified media type from the persisted artifact receipt. */
  mimeType?: string;
  insertions?: number;
  deletions?: number;
  /** Unified diff text for the latest write, when a carrier supplied one. */
  diff?: string;
  /**
   * `receipt` = committed PersistedToolArtifact (backend-verified integrity).
   * `reported` = successful edit-tool output; must pass an existence check
   * before it may be presented as an available deliverable.
   */
  tier: TurnDeliverableTier;
  sources: TurnDeliverableSource[];
}

export type VerifiedImageDeliverableItem = TurnDeliverableItem & {
  tier: 'receipt';
  artifactId: PersistedToolArtifact['id'];
  artifactKind: 'image';
  mimeType: string;
};

/**
 * The only admission path for the first-class generated-image UI. Extension
 * sniffing and assistant-authored Markdown are intentionally excluded: both
 * the artifact kind and MIME must come from a committed backend receipt.
 */
export const isVerifiedImageDeliverable = (
  item: TurnDeliverableItem
): item is VerifiedImageDeliverableItem =>
  item.tier === 'receipt' &&
  item.artifactKind === 'image' &&
  typeof item.artifactId === 'string' &&
  item.artifactId.length > 0 &&
  typeof item.mimeType === 'string' &&
  item.mimeType.toLowerCase().startsWith('image/');

type DeliverableToolMessage = IMessageToolCall | IMessageToolGroup;

export interface TurnDeliverableCandidate {
  turnId?: MessageId;
  role: TurnDisclosureRole;
  processState: TurnDisclosureProcessState;
  /** Tool messages carried by this processed item (tool_summary VO or bare message). */
  toolMessages?: DeliverableToolMessage[];
  /** Pre-parsed WriteFile diffs carried by a file_summary VO. */
  fileDiffs?: FileChangeInfo[];
  fileDiffSourceMessageIds?: MessageId[];
}

export interface TurnGateInfo {
  running: boolean;
  state: TurnDisclosureProcessState;
}

export interface CollectTurnDeliverablesOptions {
  workspaceRoots?: Array<string | null | undefined>;
  /** Per-turn lifecycle from the disclosure model; turns without an entry never emit a card. */
  turnGates: ReadonlyMap<string, TurnGateInfo>;
}

const normalizeSlashes = (value: string): string => value.trim().replace(/\\/g, '/').replace(/\/{2,}/g, '/');

const isAbsolutePath = (value: string): boolean => value.startsWith('/') || /^[A-Za-z]:\//.test(value);

const stripTrailingSlash = (value: string): string => value.replace(/\/+$/, '');

const isWindowsAddress = (value: string): boolean =>
  /^[A-Za-z]:[\\/]/.test(value.trim()) || /^(?:\\\\|\/\/)/.test(value.trim());

/**
 * A comparison-only address. Keep the canonical path itself untouched: it is
 * the backend receipt's proof-bearing locator and may use UNC or extended-path
 * syntax. POSIX addresses remain case-sensitive while Windows addresses use
 * their native case-insensitive comparison semantics.
 */
const pathIdentityKey = (value: string): string => {
  const windows = isWindowsAddress(value);
  const comparable = normalizeSlashes(value);
  return `${windows ? 'windows' : 'posix'}:${windows ? comparable.toLowerCase() : comparable}`;
};

const singleWorkspaceRoot = (
  workspaceRoots: Array<string | null | undefined>
): string | undefined => {
  const roots = [
    ...new Set(
      workspaceRoots
        .filter((root): root is string => Boolean(root?.trim()))
        .map((root) => stripTrailingSlash(root.trim()))
    ),
  ];
  return roots.length === 1 ? roots[0] : undefined;
};

const deliverableAddressKey = (
  relativePath: string,
  absolutePath: string | undefined,
  identityPath: string | undefined,
  workspaceRoots: Array<string | null | undefined>
): string => {
  const inferredRoot = absolutePath ? undefined : singleWorkspaceRoot(workspaceRoots);
  const address =
    absolutePath ??
    identityPath ??
    (inferredRoot ? `${inferredRoot}/${relativePath}` : relativePath);
  return pathIdentityKey(address);
};

interface NormalizedDeliverablePath {
  relativePath: string;
  absolutePath?: string;
  /** Stable dedupe identity when the display path intentionally hides an absolute root. */
  identityPath?: string;
}

/**
 * Windows paths compare case-insensitively for the workspace-root prefix, but
 * the remainder keeps its original casing for display.
 */
const normalizeDeliverablePath = (
  raw: string,
  workspaceRoots: Array<string | null | undefined>
): NormalizedDeliverablePath | undefined => {
  const rawPath = raw.trim();
  const pathUsesWindowsSemantics = isWindowsAddress(rawPath);
  const normalized = normalizeSlashes(rawPath);
  if (!normalized || normalized === '/' || /^[A-Za-z]:\/?$/.test(normalized)) return undefined;

  if (isAbsolutePath(normalized)) {
    for (const root of workspaceRoots) {
      if (!root) continue;
      const rawRoot = root.trim();
      if (isWindowsAddress(rawRoot) !== pathUsesWindowsSemantics) continue;
      const comparableRoot = stripTrailingSlash(normalizeSlashes(rawRoot));
      if (!comparableRoot) continue;
      const prefixMatches = pathUsesWindowsSemantics
        ? normalized.slice(0, comparableRoot.length).toLowerCase() ===
          comparableRoot.toLowerCase()
        : normalized.slice(0, comparableRoot.length) === comparableRoot;
      if (
        normalized.length > comparableRoot.length + 1 &&
        prefixMatches &&
        normalized[comparableRoot.length] === '/'
      ) {
        return { relativePath: normalized.slice(comparableRoot.length + 1), absolutePath: normalized };
      }
    }
    // The deliverables card is scoped to the current conversation, so never
    // expose a host absolute path when the backend target sits outside the
    // active workspace. Keep the absolute path only for file actions and use
    // the basename as the safest relative display fallback.
    return {
      relativePath: normalized.split('/').at(-1) ?? normalized,
      absolutePath: normalized,
      identityPath: normalized,
    };
  }

  // Reject traversal in relative paths — a display path must never suggest a
  // location outside the workspace it is presented against.
  if (normalized.split('/').includes('..')) return undefined;
  return { relativePath: normalized.replace(/^\.\//, '') };
};

const getPathBasename = (value: string): string => {
  const parts = normalizeSlashes(value).split('/').filter(Boolean);
  return parts.at(-1) ?? value;
};

/**
 * Native file-mutation tools whose successful run implies the args-declared
 * target file now exists. Matched on the compacted lowercase leaf name so
 * `ApplyPatch`, `apply_patch` and `Multi Edit` all resolve; MCP tools are
 * excluded — their names are server-controlled metadata, not trusted verbs.
 */
const FILE_MUTATION_TOOL_NAMES = new Set(['write', 'edit', 'applypatch', 'multiedit', 'patch', 'replace', 'writefile']);

const isFileMutationToolName = (name: unknown): boolean => {
  if (typeof name !== 'string') return false;
  if (name.startsWith('mcp__')) return false;
  const compacted = name.toLowerCase().replace(/[^a-z]/g, '');
  return FILE_MUTATION_TOOL_NAMES.has(compacted);
};

const collectArgsTargetPaths = (args: Record<string, unknown> | null | undefined): string[] => {
  if (!args || typeof args !== 'object') return [];
  const paths: string[] = [];

  const direct = args.file_path ?? args.path;
  if (typeof direct === 'string' && direct.trim()) paths.push(direct);

  if (Array.isArray(args.files)) {
    for (const entry of args.files) {
      if (!entry || typeof entry !== 'object') continue;
      const record = entry as Record<string, unknown>;
      if (record.delete === true) continue;
      const target = record.file_path ?? record.path;
      if (typeof target === 'string' && target.trim()) paths.push(target);
    }
  }

  return paths;
};

interface DeliverableDraft {
  path: string;
  /** Carrier-supplied canonical absolute path, when distinct from `path`. */
  absolutePath?: string;
  tier: TurnDeliverableTier;
  source: TurnDeliverableSource;
  sizeBytes?: number;
  sha256?: string;
  artifactId?: PersistedToolArtifact['id'];
  artifactKind?: PersistedToolArtifact['kind'];
  mimeType?: string;
  insertions?: number;
  deletions?: number;
  diff?: string;
}

const draftFromArtifact = (
  artifact: PersistedToolArtifact,
  carrier: TurnDeliverableCarrier,
  callId: string | undefined,
  sourceMessageIds: MessageId[]
): DeliverableDraft => ({
  path: artifact.relative_path || artifact.path,
  // `artifact.path` is a backend-issued canonical filesystem address. Keep it
  // byte-for-byte (apart from surrounding whitespace) for native actions:
  // slash normalization corrupts UNC and Windows extended-length prefixes.
  ...(artifact.path ? { absolutePath: artifact.path.trim() } : {}),
  tier: 'receipt',
  sizeBytes: artifact.size_bytes,
  sha256: artifact.sha256,
  artifactId: artifact.id,
  artifactKind: artifact.kind,
  mimeType: artifact.mime_type,
  source: { carrier, ...(callId ? { callId } : {}), sourceMessageIds },
});

const messageSourceIds = (message: DeliverableToolMessage): MessageId[] => {
  const businessId = message.message_id ?? message.msg_id;
  return businessId ? [businessId] : [];
};

const draftsFromToolCall = (message: IMessageToolCall): DeliverableDraft[] => {
  const content = message.content;
  if (content.status !== 'completed') return [];
  const sourceIds = messageSourceIds(message);
  const drafts: DeliverableDraft[] = [];

  for (const artifact of content.artifacts ?? []) {
    drafts.push(draftFromArtifact(artifact, 'tool_call_artifact', content.call_id, sourceIds));
  }

  if (isFileMutationToolName(content.name)) {
    for (const path of collectArgsTargetPaths(content.args)) {
      drafts.push({
        path,
        tier: 'reported',
        source: { carrier: 'tool_call_args', callId: content.call_id, sourceMessageIds: sourceIds },
      });
    }
  }

  return drafts;
};

const draftsFromToolGroup = (message: IMessageToolGroup): DeliverableDraft[] => {
  if (!Array.isArray(message.content)) return [];
  const sourceIds = messageSourceIds(message);
  const drafts: DeliverableDraft[] = [];

  for (const entry of message.content) {
    if (!isSuccessfulWriteFileResult(entry)) continue;
    const display = entry.result_display as WriteFileResult;
    const info = parseDiff(display.file_diff, display.file_name);
    drafts.push({
      path: info.fullPath,
      tier: 'reported',
      insertions: info.insertions,
      deletions: info.deletions,
      diff: info.diff,
      source: { carrier: 'tool_group_write_file', callId: entry.call_id, sourceMessageIds: sourceIds },
    });
  }

  return drafts;
};

const draftsFromCandidate = (candidate: TurnDeliverableCandidate): DeliverableDraft[] => {
  const drafts: DeliverableDraft[] = [];

  for (const message of candidate.toolMessages ?? []) {
    if (message.type === 'tool_call') drafts.push(...draftsFromToolCall(message));
    else if (message.type === 'tool_group') drafts.push(...draftsFromToolGroup(message));
  }

  for (const info of candidate.fileDiffs ?? []) {
    drafts.push({
      path: info.fullPath || info.file_name,
      tier: 'reported',
      insertions: info.insertions,
      deletions: info.deletions,
      diff: info.diff,
      source: { carrier: 'write_file_diff', sourceMessageIds: candidate.fileDiffSourceMessageIds ?? [] },
    });
  }

  return drafts;
};

const mergeDraft = (
  byPath: Map<string, TurnDeliverableItem>,
  order: string[],
  draft: DeliverableDraft,
  workspaceRoots: Array<string | null | undefined>
): void => {
  const normalized = normalizeDeliverablePath(draft.path, workspaceRoots);
  if (!normalized) return;

  const absolutePath = draft.absolutePath ?? normalized.absolutePath;
  const addressKey = deliverableAddressKey(
    normalized.relativePath,
    absolutePath,
    normalized.identityPath,
    workspaceRoots
  );
  // A backend artifact id is the receipt identity. Path-based drafts live in
  // a separate namespace and are reconciled only after all evidence is known,
  // so a reported path can never become part of a receipt proof object.
  const key =
    draft.tier === 'receipt' && draft.artifactId
      ? `receipt:${draft.artifactId}`
      : `path:${addressKey}`;
  const existing = byPath.get(key);

  if (!existing) {
    byPath.set(key, {
      relativePath: normalized.relativePath,
      fileName: getPathBasename(normalized.relativePath),
      ...(absolutePath ? { absolutePath } : {}),
      ...(draft.sizeBytes !== undefined ? { sizeBytes: draft.sizeBytes } : {}),
      ...(draft.sha256 !== undefined ? { sha256: draft.sha256 } : {}),
      ...(draft.artifactId !== undefined ? { artifactId: draft.artifactId } : {}),
      ...(draft.artifactKind !== undefined ? { artifactKind: draft.artifactKind } : {}),
      ...(draft.mimeType !== undefined ? { mimeType: draft.mimeType } : {}),
      ...(draft.insertions !== undefined ? { insertions: draft.insertions } : {}),
      ...(draft.deletions !== undefined ? { deletions: draft.deletions } : {}),
      ...(draft.diff !== undefined ? { diff: draft.diff } : {}),
      tier: draft.tier,
      sources: [draft.source],
    });
    order.push(key);
    return;
  }

  // Receipt identity, locator, hash, MIME and size form one indivisible proof
  // object. A later unverified write/report may contribute provenance or a
  // diff, but it can never replace any proof-bearing field.
  existing.sources.push(draft.source);
  if (draft.insertions !== undefined) existing.insertions = draft.insertions;
  if (draft.deletions !== undefined) existing.deletions = draft.deletions;
  if (draft.diff !== undefined) existing.diff = draft.diff;

  if (existing.tier === 'receipt') {
    return;
  }

  if (draft.tier === 'receipt') {
    // Promote reported metadata to a receipt atomically. The canonical receipt
    // address replaces (rather than mixes with) the reported draft address.
    existing.tier = 'receipt';
    existing.relativePath = normalized.relativePath;
    existing.fileName = getPathBasename(normalized.relativePath);
    if (absolutePath) existing.absolutePath = absolutePath;
    else delete existing.absolutePath;
    if (draft.sizeBytes !== undefined) existing.sizeBytes = draft.sizeBytes;
    else delete existing.sizeBytes;
    if (draft.sha256 !== undefined) existing.sha256 = draft.sha256;
    else delete existing.sha256;
    if (draft.artifactId !== undefined) existing.artifactId = draft.artifactId;
    else delete existing.artifactId;
    if (draft.artifactKind !== undefined) existing.artifactKind = draft.artifactKind;
    else delete existing.artifactKind;
    if (draft.mimeType !== undefined) existing.mimeType = draft.mimeType;
    else delete existing.mimeType;
  } else {
    if (absolutePath) existing.absolutePath = absolutePath;
    if (draft.sizeBytes !== undefined) existing.sizeBytes = draft.sizeBytes;
    if (draft.sha256 !== undefined) existing.sha256 = draft.sha256;
  }
};

/**
 * Aggregate the verified file deliverables of every successfully closed turn.
 *
 * The invariants enforced here mirror the backend's delivery semantics:
 * running/waiting turns emit nothing, canceled turns emit nothing, and a turn
 * whose final observed item failed emits nothing. A mid-turn failure that the
 * agent recovered from does not disqualify the turn — failed tool calls simply
 * contribute no drafts.
 */
export function collectTurnDeliverables(
  candidates: TurnDeliverableCandidate[],
  options: CollectTurnDeliverablesOptions
): Map<MessageId, TurnDeliverableItem[]> {
  const workspaceRoots = options.workspaceRoots ?? [];
  const draftsByTurn = new Map<MessageId, DeliverableDraft[]>();
  const terminalStateByTurn = new Map<MessageId, TurnDisclosureProcessState>();

  for (const candidate of candidates) {
    const turnId = candidate.turnId;
    if (!turnId) continue;

    if (candidate.role !== 'user' && candidate.role !== 'other') {
      terminalStateByTurn.set(turnId, candidate.processState);
    }

    const drafts = draftsFromCandidate(candidate);
    if (!drafts.length) continue;
    const bucket = draftsByTurn.get(turnId);
    if (bucket) bucket.push(...drafts);
    else draftsByTurn.set(turnId, [...drafts]);
  }

  const result = new Map<MessageId, TurnDeliverableItem[]>();

  for (const [turnId, drafts] of draftsByTurn) {
    const gate = options.turnGates.get(turnId);
    if (!gate || gate.running) continue;
    if (gate.state === 'canceled' || gate.state === 'failed' || gate.state === 'running' || gate.state === 'waiting') {
      continue;
    }
    const terminalState = terminalStateByTurn.get(turnId);
    if (terminalState === 'failed' || terminalState === 'canceled') continue;

    const byPath = new Map<string, TurnDeliverableItem>();
    const order: string[] = [];
    for (const draft of drafts) {
      mergeDraft(byPath, order, draft, workspaceRoots);
    }

    const items = order
      .map((key) => byPath.get(key))
      .filter((item): item is TurnDeliverableItem => Boolean(item));
    const receiptsByAddress = new Map<string, TurnDeliverableItem[]>();
    for (const item of items) {
      if (item.tier !== 'receipt') continue;
      const address = deliverableAddressKey(
        item.relativePath,
        item.absolutePath,
        undefined,
        workspaceRoots
      );
      const bucket = receiptsByAddress.get(address);
      if (bucket) bucket.push(item);
      else receiptsByAddress.set(address, [item]);
    }

    const reconciled: TurnDeliverableItem[] = [];
    for (const item of items) {
      if (item.tier === 'reported') {
        const address = deliverableAddressKey(
          item.relativePath,
          item.absolutePath,
          undefined,
          workspaceRoots
        );
        const matchingReceipts = receiptsByAddress.get(address) ?? [];
        if (matchingReceipts.length === 1) {
          const receipt = matchingReceipts[0];
          receipt.sources.push(...item.sources);
          if (item.insertions !== undefined) receipt.insertions = item.insertions;
          if (item.deletions !== undefined) receipt.deletions = item.deletions;
          if (item.diff !== undefined) receipt.diff = item.diff;
          continue;
        }
      }
      reconciled.push(item);
    }
    if (reconciled.length) result.set(turnId, reconciled);
  }

  return result;
}
