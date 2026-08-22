/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeStudioAgentMessage,
  CreativeStudioAgentProposal,
} from '../../../agent';
import {
  parseCreativeCanvasAgentArtifact,
  type CreativeCanvasAgentArtifact,
} from './artifacts';

export interface CreativeCanvasProposalOverride {
  state: 'applying' | 'applied' | 'failed';
  errorMessage?: string;
}

export interface CreativeCanvasProposalProjection {
  artifacts: ReadonlyMap<string, CreativeCanvasAgentArtifact>;
  proposals: readonly CreativeStudioAgentProposal[];
}

/** Project strict artifacts plus durable receipt authority into card state. */
export function projectCreativeCanvasAgentProposals(
  messages: readonly CreativeStudioAgentMessage[],
  overrides: Readonly<Record<string, CreativeCanvasProposalOverride>>,
  appliedProposalMessageIds: readonly string[]
): CreativeCanvasProposalProjection {
  const artifacts = new Map<string, CreativeCanvasAgentArtifact>();
  const proposals: CreativeStudioAgentProposal[] = [];
  const applied = new Set(appliedProposalMessageIds);
  for (const message of messages) {
    if (message.role !== 'assistant' || message.status !== 'complete') continue;
    try {
      const artifact = parseCreativeCanvasAgentArtifact(message.text);
      if (!artifact) continue;
      artifacts.set(message.id, artifact);
      const override = overrides[message.id];
      const authoritativelyApplied = applied.has(message.id);
      proposals.push({
        messageId: message.id,
        summary: artifact.summary,
        opCount: artifact.ops.length,
        state: authoritativelyApplied ? 'applied' : (override?.state ?? 'ready'),
        ...(!authoritativelyApplied && override?.errorMessage
          ? { errorMessage: override.errorMessage }
          : {}),
      });
    } catch {
      proposals.push({
        messageId: message.id,
        summary: 'Agent 返回的画布提案格式无效',
        opCount: 0,
        state: 'invalid',
        errorMessage: '该提案未通过严格合同校验，不能应用到画布。',
      });
    }
  }
  return { artifacts, proposals };
}
