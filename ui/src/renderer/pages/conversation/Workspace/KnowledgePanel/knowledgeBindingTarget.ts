/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ITerminalSession } from '@/common/adapter/ipcBridge';
import type { KnowledgeBindingKind } from '@/common/adapter/ipcBridge';
import type { ConversationId } from '@/common/types/ids';
import {
  workpathKeyForConversation,
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
      conversationId: ConversationId;
      extra: Record<string, unknown> | undefined;
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

/**
 * Which knowledge-binding row a session actually reads and writes.
 *
 * Mirrors the backend exactly — `knowledge_binding_target`
 * (crates/backend/nomifun-conversation/src/service.rs) picks the target kind,
 * then the mount dispatcher (same file, `prepare_mounts_for_session` vs
 * `prepare_mounts_for_target`) decides whether a conversation target collapses
 * to its workpath:
 *
 * 1. `extra.companion_id` present     → ('companion', companion_id)
 * 2. `extra.preset_knowledge_binding` → ('conversation', conversation_id)
 * 3. otherwise                        → ('workpath', workpathKey(workspace))
 *
 * Branch 2 is the one `KnowledgeControl`'s inline memo is missing, which is why
 * this lives in its own tested function instead of being copied again.
 */
export function resolveKnowledgeBindingTarget(source: SessionKnowledgeSource): ResolvedKnowledgeBindingTarget {
  if (source.kind === 'terminal') {
    return { kind: 'workpath', target_id: workpathKeyForTerminal(source.session) };
  }

  const extra = source.extra ?? {};

  const companionId = extra.companion_id;
  if (typeof companionId === 'string' && companionId.trim().length > 0) {
    return { kind: 'companion', target_id: companionId };
  }

  if (extra.preset_knowledge_binding === true) {
    return { kind: 'conversation', target_id: source.conversationId };
  }

  return { kind: 'workpath', target_id: workpathKeyForConversation(extra) };
}

/** Stable cache/subscription key for a resolved target. */
export function knowledgeBindingTargetKey(target: ResolvedKnowledgeBindingTarget): string {
  return `${target.kind}:${target.target_id}`;
}
