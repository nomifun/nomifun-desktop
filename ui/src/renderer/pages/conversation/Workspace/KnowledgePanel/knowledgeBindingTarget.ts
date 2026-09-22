/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import type { KnowledgeBindingKind } from '@/common/adapter/ipcBridge';
import type { ConversationId } from '@/common/types/ids';
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
      /** Canonical AgentSession whose Knowledge subset is mutable between turns. */
      sessionId: ConversationId;
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
  kind: KnowledgeBindingKind | 'conversation';
  target_id: string;
}

/** Only workpath-backed sessions have a workspace scope to display. */
export function workpathDisplayForKnowledgeTarget(
  target: ResolvedKnowledgeBindingTarget | null | undefined
): string | null {
  return target?.kind === 'workpath' ? target.target_id : null;
}

/**
 * Resolve the product-owned binding address. Conversation updates use the
 * dedicated AgentSession command; terminals keep the workpath binding route.
 */
export function resolveKnowledgeBindingTarget(
  source: SessionKnowledgeSource
): ResolvedKnowledgeBindingTarget | null {
  if (source.kind === 'terminal') {
    return { kind: 'workpath', target_id: workpathKeyForTerminal(source.session) };
  }
  return { kind: 'conversation', target_id: source.sessionId };
}

/** Stable cache/subscription key for a resolved target. */
export function knowledgeBindingTargetKey(target: ResolvedKnowledgeBindingTarget): string {
  return `${target.kind}:${target.target_id}`;
}
