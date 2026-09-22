/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import type { KnowledgeBindingKind } from '@/common/adapter/ipcBridge';
import type { KnowledgeBaseId } from '@/common/types/ids';
import {
  workpathKeyForTerminal,
} from '@/renderer/pages/conversation/SessionList/utils/sessionWorkpath';

/**
 * The session whose mounted knowledge bases we want to read.
 *
 * Deliberately takes the raw session object rather than an id: a terminal owned
 * by a conversation is filtered out of `useTerminalSessions()` (see
 * `pages/terminal/useTerminalSessions.ts`), so an id-plus-lookup resolution
 * silently fails for those. Every caller already holds the real object.
 */
export type SessionKnowledgeSource =
  | {
      kind: 'conversation';
      /** Exact resources frozen into the canonical AgentSession binding. */
      knowledgeBaseIds: readonly KnowledgeBaseId[];
    }
  | {
      kind: 'terminal';
      session: Pick<ITerminalSession, 'cwd' | 'is_default_workpath'>;
    };

/**
 * A resolved binding row address. Named distinctly from `ipcBridge`'s
 * `KnowledgeBindingTarget` (which carries branded ids per kind) because this is
 * the widened, already-resolved form the read path passes around.
 */
export interface ResolvedKnowledgeBindingTarget {
  kind: KnowledgeBindingKind;
  target_id: string;
}

/** Only workpath-backed sessions have a workspace scope to display. */
export function workpathDisplayForKnowledgeTarget(
  target: ResolvedKnowledgeBindingTarget | null | undefined
): string | null {
  return target?.kind === 'workpath' ? target.target_id : null;
}

/**
 * Resolve only mutable workpath-backed terminal bindings. Conversation
 * resources are already carried by their frozen canonical AgentSession and do
 * not have a second Knowledge-binding row.
 */
export function resolveKnowledgeBindingTarget(
  source: SessionKnowledgeSource
): ResolvedKnowledgeBindingTarget | null {
  if (source.kind === 'terminal') {
    return { kind: 'workpath', target_id: workpathKeyForTerminal(source.session) };
  }
  return null;
}

/** Stable cache/subscription key for a resolved target. */
export function knowledgeBindingTargetKey(target: ResolvedKnowledgeBindingTarget): string {
  return `${target.kind}:${target.target_id}`;
}
