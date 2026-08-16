/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ICronJob } from '@/common/adapter/ipcBridge';
import { getAgentLogo } from '@renderer/utils/model/agentLogo';
import type { AgentMetadata } from '@renderer/utils/model/agentTypes';

function normalizeAgentBackend(agent: string | undefined): string | undefined {
  if (!agent) return undefined;
  return agent.replace(/^cli:/, '').replace(/^preset:/, '');
}

/**
 * Resolve the display name and logo for a cron job's agent.
 *
 * ACP jobs store the literal string "acp" in `agent_type`; the real vendor id
 * (claude/gemini/codex/…) and the human-readable label live in `agent_config`.
 * Non-ACP agents (nomi, remote, openclaw-gateway, …) use
 * `agent_type` directly. Nomi's provider_id is a model selection and is not
 * used to resolve the agent logo.
 */
export function getJobAgentMeta(job: ICronJob, cliAgents: AgentMetadata[]): { name?: string; logo?: string | null } {
  const rawType = normalizeAgentBackend(job.metadata.agent_type);
  if (!rawType) return {};
  const config = job.metadata.agent_config;
  // A preset is the user-selected execution identity. Keep its frozen name
  // visible even when the underlying runtime resolves to Claude/Codex/etc.
  const presetName = config?.preset_id ? config.name.trim() : undefined;
  const hasStableAgentId = Boolean(config?.custom_agent_id);
  const detectedById = config?.custom_agent_id
    ? cliAgents.find((agent) => agent.agent_id === config.custom_agent_id)
    : undefined;

  if (rawType === 'acp') {
    const backend = config?.backend;
    // Once a stable identity exists, never substitute another Agent that happens
    // to share its backend. Deleted Agents retain their frozen configured name.
    const detected = hasStableAgentId
      ? detectedById
      : backend
        ? cliAgents.find((agent) => (agent.backend || agent.agent_type) === backend)
        : undefined;
    return {
      name: presetName || detected?.name || config?.name || backend || rawType,
      logo: getAgentLogo(detected?.backend || backend),
    };
  }

  const detected = hasStableAgentId
    ? detectedById
    : cliAgents.find((agent) => (agent.backend || agent.agent_type) === rawType);
  return {
    name: presetName || detected?.name || config?.name || rawType,
    logo: getAgentLogo(detected?.backend || rawType),
  };
}
