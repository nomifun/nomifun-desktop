/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentSource } from '@/renderer/utils/model/agentTypes';
import type { ProviderId } from '@/common/types/ids';
import { configService } from '@/common/config/configService';

/** Save default nomi provider/model so the Guid page restores it next session. */
export async function saveNomiDefaultModel(provider_id: ProviderId, use_model: string): Promise<void> {
  try {
    await configService.set('nomi.defaultModel', { provider_id, model: use_model });
  } catch {
    /* silent */
  }
}

/**
 * Get agent key for selection.
 *
 * Rows that are row-scoped (custom agents) use `agent_id` directly
 * as the key — no namespace prefix. Builtin / internal agents keep `backend` or
 * `agent_type` as the key since there is only one row per type.
 *
 */
export const getAgentKey = (agent: {
  agent_type: string;
  agent_source?: AgentSource;
  backend?: string;
  /** Named wire identity on AgentMetadata before it enters a UI aggregate. */
  agent_id?: string;
  /** Local identity slot used by the mixed AvailableAgent display aggregate. */
  id?: string;
}): string => {
  const rowScoped = agent.agent_type === 'remote' || agent.agent_source === 'custom';
  const rowIdentity = agent.agent_id ?? agent.id;
  if (rowScoped && rowIdentity) return rowIdentity;
  return agent.backend || agent.agent_type;
};
