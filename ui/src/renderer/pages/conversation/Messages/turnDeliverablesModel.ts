/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  IMessageAcpToolCall,
  IMessageToolCall,
  IMessageToolGroup,
} from '@/common/chat/chatLib';
import type { PersistedToolArtifact } from '@/common/types/platform/acpTypes';
import type { MessageId } from '@/common/types/ids';
import { parseDiff, type FileChangeInfo } from '@/renderer/utils/file/diffUtils';
import { createTwoFilesPatch } from 'diff';
import { isSuccessfulWriteFileResult } from './components/toolGroupArtifactVisibility';
import type { TurnDisclosureProcessState, TurnDisclosureRole } from './turnDisclosureModel';
import type { WriteFileResult } from './types';

export type TurnDeliverableTier = 'receipt' | 'reported';

export type TurnDeliverableCarrier =
  | 'tool_call_artifact'
  | 'tool_call_args'
  | 'acp_artifact'
  | 'acp_diff'
  | 'acp_edit_target'
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

type DeliverableToolMessage = IMessageToolCall | IMessageAcpToolCall | IMessageToolGroup;

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
  const normalized = normalizeSlashes(raw);
  if (!normalized || normalized === '/' || /^[A-Za-z]:\/?$/.test(normalized)) return undefined;

  if (isAbsolutePath(normalized)) {
    for (const root of workspaceRoots) {
      if (!root) continue;
      const comparableRoot = stripTrailingSlash(normalizeSlashes(root));
      if (!comparableRoot) continue;
      if (
        normalized.length > comparableRoot.length + 1 &&
        normalized.slice(0, comparableRoot.length).toLowerCase() === comparableRoot.toLowerCase() &&
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

const countPatchLines = (patch: string): { insertions: number; deletions: number } => {
  let insertions = 0;
  let deletions = 0;
  for (const line of patch.split('\n')) {
    if (line.startsWith('+++') || line.startsWith('---') || line.startsWith('@@') || line.startsWith('\\')) continue;
    if (line.startsWith('+')) insertions += 1;
    else if (line.startsWith('-')) deletions += 1;
  }
  return { insertions, deletions };
};

interface DeliverableDraft {
  path: string;
  /** Carrier-supplied canonical absolute path, when distinct from `path`. */
  absolutePath?: string;
  tier: TurnDeliverableTier;
  source: TurnDeliverableSource;
  sizeBytes?: number;
  sha256?: string;
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
  ...(artifact.path ? { absolutePath: normalizeSlashes(artifact.path) } : {}),
  tier: 'receipt',
  sizeBytes: artifact.size_bytes,
  sha256: artifact.sha256,
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

const draftsFromAcpToolCall = (message: IMessageAcpToolCall): DeliverableDraft[] => {
  const update = message.content?.update;
  if (!update || update.status !== 'completed') return [];
  const sourceIds = messageSourceIds(message);
  const callId = update.tool_call_id;
  const drafts: DeliverableDraft[] = [];

  for (const item of update.content ?? []) {
    if (item.type === 'artifact') {
      drafts.push(draftFromArtifact(item.artifact, 'acp_artifact', callId, sourceIds));
      continue;
    }
    if (item.type === 'diff' && typeof item.path === 'string' && item.path.trim()) {
      const displayName = getPathBasename(item.path);
      const oldText = typeof item.old_text === 'string' ? item.old_text : '';
      const newText = typeof item.new_text === 'string' ? item.new_text : '';
      const patch = createTwoFilesPatch(displayName, displayName, oldText, newText, '', '', { context: 3 });
      drafts.push({
        path: item.path,
        tier: 'reported',
        ...countPatchLines(patch),
        diff: patch,
        source: { carrier: 'acp_diff', callId, sourceMessageIds: sourceIds },
      });
    }
  }

  if (update.kind === 'edit') {
    const targets = new Set<string>();
    const rawInput = update.rawInput;
    if (rawInput && typeof rawInput === 'object') {
      const direct = (rawInput as Record<string, unknown>).file_path ?? (rawInput as Record<string, unknown>).path;
      if (typeof direct === 'string' && direct.trim()) targets.add(direct);
    }
    for (const location of update.locations ?? []) {
      if (typeof location?.path === 'string' && location.path.trim()) targets.add(location.path);
    }
    for (const path of targets) {
      drafts.push({
        path,
        tier: 'reported',
        source: { carrier: 'acp_edit_target', callId, sourceMessageIds: sourceIds },
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
    else if (message.type === 'acp_tool_call') drafts.push(...draftsFromAcpToolCall(message));
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

  const key = (normalized.identityPath ?? normalized.relativePath).toLowerCase();
  const absolutePath = draft.absolutePath ?? normalized.absolutePath;
  const existing = byPath.get(key);
  if (!existing) {
    byPath.set(key, {
      relativePath: normalized.relativePath,
      fileName: getPathBasename(normalized.relativePath),
      ...(absolutePath ? { absolutePath } : {}),
      ...(draft.sizeBytes !== undefined ? { sizeBytes: draft.sizeBytes } : {}),
      ...(draft.sha256 !== undefined ? { sha256: draft.sha256 } : {}),
      ...(draft.insertions !== undefined ? { insertions: draft.insertions } : {}),
      ...(draft.deletions !== undefined ? { deletions: draft.deletions } : {}),
      ...(draft.diff !== undefined ? { diff: draft.diff } : {}),
      tier: draft.tier,
      sources: [draft.source],
    });
    order.push(key);
    return;
  }

  // Same file written again later in the turn: the final version wins, receipt
  // evidence outranks reported evidence, and provenance accumulates.
  existing.sources.push(draft.source);
  if (absolutePath) existing.absolutePath = absolutePath;
  if (draft.insertions !== undefined) existing.insertions = draft.insertions;
  if (draft.deletions !== undefined) existing.deletions = draft.deletions;
  if (draft.diff !== undefined) existing.diff = draft.diff;
  if (draft.tier === 'receipt') {
    existing.tier = 'receipt';
    if (draft.sizeBytes !== undefined) existing.sizeBytes = draft.sizeBytes;
    if (draft.sha256 !== undefined) existing.sha256 = draft.sha256;
  } else if (existing.tier !== 'receipt') {
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
    if (items.length) result.set(turnId, items);
  }

  return result;
}
