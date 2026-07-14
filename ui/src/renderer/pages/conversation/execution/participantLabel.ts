/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TExecutionParticipant } from '@/common/types/agentExecution/agentExecutionTypes';
import { resolveAgentLogo } from '@/renderer/utils/model/agentLogo';

/** Build a stable label from a participant's role, source, and model snapshot. */
export function participantShortLabel(participant: TExecutionParticipant | undefined): string | null {
  if (!participant) return null;
  const agent = participant.role?.trim() || participant.source_agent_id;
  const model = participant.model?.trim();
  if (!agent && !model) return null;
  if (agent && model) return `${agent} · ${model}`;
  return agent || model || null;
}

/** Resolve the logo for an execution participant. */
export function participantLogo(participant: TExecutionParticipant | undefined): string | null {
  if (!participant?.source_agent_id) return null;
  return resolveAgentLogo({ backend: participant.source_agent_id });
}
