/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AcpModelInfo } from '@/common/types/platform/acpTypes';
import type { AgentSource } from '@/renderer/utils/model/agentTypes';
import type { PresetReference } from '@/common/types/agent/presetTypes';
import type { RemoteAgentId } from '@/common/types/ids';

/**
 * Available agent entry returned by the backend.
 * `agent_type` is the top-level discriminant (acp, nomi, nanobot, etc.).
 * `backend` is only present when `agent_type === 'acp'` (claude, qwen, codex, …).
 */
export type AvailableAgent = {
  /**
   * Stable identity. For `agent_source === 'custom'` or `agent_type === 'remote'`
   * this is the row id that discriminates between rows with the same
   * `agent_type` / `backend`. Canonical field for agent selection going
   * forward — prefer `id` over `custom_agent_id`.
   */
  id?: string;
  agent_type: string;
  agent_source?: AgentSource;
  backend?: string;
  icon?: string;
  name: string;
  cli_path?: string;
  /**
   * @deprecated Alias for `id` retained for downstream consumers
   * (preset resolver / send hook / mention tokens). Will be removed in
   * a follow-up PR. Always equals `id` when populated.
   */
  custom_agent_id?: string;
  /** Canonical remote-agent entity identity; never routed through the custom-agent catalog key. */
  remote_agent_id?: RemoteAgentId;
  is_preset?: boolean;
  preset_id?: PresetReference;
  context?: string;
  avatar?: string;
  isExtension?: boolean;
  extensionName?: string;
};

/**
 * Computed mention option for the @ mention dropdown.
 */
export type MentionOption = {
  key: string;
  label: string;
  tokens: Set<string>;
  avatar: string | undefined;
  avatarImage: string | undefined;
  logo: string | undefined;
  isExtension?: boolean;
};

/**
 * Effective agent type info used for UI display and send logic.
 */
export type EffectiveAgentInfo = {
  agent_type: string;
  isFallback: boolean;
  originalType: string;
  isAvailable: boolean;
};

export type { AcpModelInfo };
