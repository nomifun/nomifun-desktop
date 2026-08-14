/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ICronJob } from '@/common/adapter/ipcBridge';
import type { PresetReference } from '@/common/types/agent/presetTypes';
import type { AgentId } from '@/common/types/ids';
import { resolveLocaleKey } from '@/common/utils';
import type { AgentMetadata } from '@/renderer/utils/model/agentTypes';

const AGENT_PREFIX = 'agent:';
const PRESET_PREFIX = 'preset:';
const LEGACY_PREFIX = 'legacy:';

/** Select values are UI-only identities. Persisted cron configuration uses the parsed IDs. */
export const getCronAgentOptionValue = (agentId: AgentId): string => `${AGENT_PREFIX}${agentId}`;

export const getCronPresetOptionValue = (presetId: PresetReference): string => `${PRESET_PREFIX}${presetId}`;

const getCronLegacyOptionValue = (identity: string): string => `${LEGACY_PREFIX}${identity}`;

type CronAgentSelection =
  | { kind: 'agent'; id: string }
  | { kind: 'preset'; id: string }
  | { kind: 'legacy'; id: string };

export const parseCronAgentSelection = (value: string | undefined): CronAgentSelection | undefined => {
  if (!value) return undefined;
  const separator = value.indexOf(':');
  if (separator < 0) return { kind: 'legacy', id: value };

  const kind = value.slice(0, separator);
  const id = value.slice(separator + 1);
  if (!id) return undefined;
  if (kind === 'agent') return { kind: 'agent', id };
  if (kind === 'preset') return { kind: 'preset', id };
  // `cli:<backend>` was used before AgentRegistry UUIDs were restored to this selector.
  if (kind === 'cli' || kind === 'legacy') return { kind: 'legacy', id };
  return undefined;
};

export const findCronSelectedAgent = (
  value: string | undefined,
  agents: AgentMetadata[]
): AgentMetadata | undefined => {
  const selection = parseCronAgentSelection(value);
  if (!selection || selection.kind === 'preset') return undefined;
  if (selection.kind === 'agent') return agents.find((agent) => agent.agent_id === selection.id);

  const matches = agents.filter((agent) => (agent.backend || agent.agent_type) === selection.id);
  return matches.length === 1 ? matches[0] : undefined;
};

/**
 * Restore edit-mode selection without conflating several custom ACP agents.
 * New jobs always have `custom_agent_id`; the backend/type fallback is only for legacy rows and
 * is accepted when it identifies exactly one current AgentRegistry entry.
 */
export const getCronAgentSelectionFromJob = (
  job: ICronJob,
  agents: AgentMetadata[]
): string | undefined => {
  const config = job.metadata.agent_config;
  if (config?.preset_id) return getCronPresetOptionValue(config.preset_id);
  if (config?.custom_agent_id) return getCronAgentOptionValue(config.custom_agent_id);

  const legacyIdentity = config?.backend?.trim() || job.metadata.agent_type?.trim();
  if (!legacyIdentity) return undefined;
  const matches = agents.filter((agent) => (agent.backend || agent.agent_type) === legacyIdentity);
  return matches.length === 1
    ? getCronAgentOptionValue(matches[0].agent_id)
    : getCronLegacyOptionValue(legacyIdentity);
};

export const resolveCronAgentDisplayName = (agent: AgentMetadata, language: string): string => {
  const localeKey = resolveLocaleKey(language);
  return (
    agent.name_i18n?.[language]?.trim() ||
    agent.name_i18n?.[localeKey]?.trim() ||
    agent.name_i18n?.['en-US']?.trim() ||
    agent.name.trim() ||
    agent.agent_id
  );
};

const optionalTextEqual = (left: string | undefined, right: string | undefined): boolean =>
  (left?.trim() || undefined) === (right?.trim() || undefined);

const stringRecordEqual = (
  left: Record<string, string> | undefined,
  right: Record<string, string> | undefined
): boolean => {
  const leftEntries = Object.entries(left ?? {}).sort(([a], [b]) => a.localeCompare(b));
  const rightEntries = Object.entries(right ?? {}).sort(([a], [b]) => a.localeCompare(b));
  return (
    leftEntries.length === rightEntries.length &&
    leftEntries.every(([key, value], index) => {
      const other = rightEntries[index];
      return other?.[0] === key && other[1] === value;
    })
  );
};

type CronAgentEditorState = {
  selection?: string;
  model?: string;
  providerId?: string;
  configOptions?: Record<string, string>;
  workspace?: string;
  clearContextEachRun: boolean;
};

/** Prevent ordinary task edits from replacing a server-frozen preset snapshot. */
export const hasCronAgentConfigurationChanged = (
  job: ICronJob,
  agents: AgentMetadata[],
  current: CronAgentEditorState
): boolean => {
  const config = job.metadata.agent_config;
  return (
    current.selection !== getCronAgentSelectionFromJob(job, agents) ||
    !optionalTextEqual(current.model, config?.model) ||
    !optionalTextEqual(current.providerId, config?.provider_id) ||
    !stringRecordEqual(current.configOptions, config?.config_options) ||
    !optionalTextEqual(current.workspace, config?.workspace) ||
    current.clearContextEachRun !== (config?.clear_context_each_run ?? false)
  );
};
