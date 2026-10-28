/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IKnowledgeBinding, IKnowledgeConsumer, KnowledgeBindingKind } from '@/common/adapter/ipcBridge';
import type { KnowledgeBaseId } from '@/common/types/ids';

/** Remove one base from a binding; removing the last base turns it off. */
export function removeBaseFromBinding(binding: IKnowledgeBinding, baseId: KnowledgeBaseId): IKnowledgeBinding {
  const kbIds = binding.kb_ids.filter((id) => id !== baseId);
  return {
    ...binding,
    enabled: kbIds.length > 0 ? binding.enabled : false,
    writeback: kbIds.length > 0 ? binding.writeback : false,
    writeback_eagerness: kbIds.length > 0 ? binding.writeback_eagerness : 'manual',
    kb_ids: kbIds,
  };
}

/**
 * Live AgentSession selections are the only consumers that block base
 * deletion — the repository cascades other binding kinds' junction rows in
 * the same transaction, so they never need a manual unmount.
 */
export function isSessionConsumer(consumer: IKnowledgeConsumer): boolean {
  return consumer.target_kind === 'conversation' && !!consumer.target_id;
}

/** AgentSession consumers currently selecting the base (the delete fence). */
export function sessionConsumers(consumers: IKnowledgeConsumer[]): IKnowledgeConsumer[] {
  return consumers.filter(isSessionConsumer);
}

/**
 * Detach one consumer from the base. Conversation consumers go through the
 * canonical AgentSession Knowledge command; legacy binding kinds go through
 * the knowledge binding API.
 */
export async function unmountConsumer(consumer: IKnowledgeConsumer, baseId: KnowledgeBaseId): Promise<void> {
  if (!consumer.target_id) return;
  if (consumer.target_kind === 'conversation') {
    const binding = await ipcBridge.agentPlatform.sessions.getKnowledge.invoke({
      agent_session_id: consumer.target_id,
    });
    const next = removeBaseFromBinding({ ...binding, channel_write_enabled: false }, baseId);
    if (next.kb_ids.length !== binding.kb_ids.length || next.enabled !== binding.enabled) {
      await ipcBridge.agentPlatform.sessions.updateKnowledge.invoke({
        agent_session_id: consumer.target_id,
        binding: {
          enabled: next.enabled,
          writeback: next.writeback,
          writeback_eagerness: next.writeback_eagerness,
          kb_ids: next.kb_ids,
        },
      });
    }
    return;
  }
  const binding = await ipcBridge.knowledge.getBinding.invoke({
    kind: consumer.target_kind as KnowledgeBindingKind,
    target_id: consumer.target_id,
  });
  const next = removeBaseFromBinding(binding, baseId);
  if (next.kb_ids.length !== binding.kb_ids.length || next.enabled !== binding.enabled) {
    await ipcBridge.knowledge.setBinding.invoke({
      kind: consumer.target_kind as KnowledgeBindingKind,
      target_id: consumer.target_id,
      ...next,
    });
  }
}

/**
 * Unmount every consumer that can block base deletion, in order. A failure
 * (e.g. an active Turn in the session) aborts with that error — already
 * removed mounts stay removed, matching the per-row unmount behaviour.
 */
export async function unmountSessionConsumers(
  consumers: IKnowledgeConsumer[],
  baseId: KnowledgeBaseId
): Promise<void> {
  for (const consumer of sessionConsumers(consumers)) {
    await unmountConsumer(consumer, baseId);
  }
}
