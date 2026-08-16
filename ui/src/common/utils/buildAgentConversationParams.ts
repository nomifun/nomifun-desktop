/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ICreateConversationParams } from '@/common/adapter/ipcBridge';
import type { TProviderWithModel } from '@/common/config/storage';
import type { PresetReference } from '@/common/types/agent/presetTypes';

export type BuildAgentConversationInput = {
  backend: string;
  name: string;
  agent_id?: string;
  agent_name?: string;
  preset_id?: PresetReference;
  workspace: string;
  model: TProviderWithModel;
  cli_path?: string;
  custom_workspace?: boolean;
  is_preset?: boolean;
  session_mode?: string;
  current_model_id?: string;
  extra?: Partial<ICreateConversationParams['extra']>;
};

export function buildAgentConversationParams(input: BuildAgentConversationInput): ICreateConversationParams {
  const {
    backend,
    name,
    agent_id,
    agent_name,
    preset_id,
    workspace,
    model,
    cli_path,
    custom_workspace = true,
    is_preset = false,
    session_mode,
    current_model_id,
    extra: extraOverrides,
  } = input;

  // Only one execution engine remains; the annotation keeps TS rejecting a
  // stale literal if the union ever widens again.
  const type: ICreateConversationParams['type'] = 'nomi';
  const extra: ICreateConversationParams['extra'] = {
    workspace,
    custom_workspace,
    ...extraOverrides,
  };

  // Bare Agent launches carry their runtime identity in `extra`; a preset
  // launch resolves everything server-side from `preset_id` instead.
  if (!is_preset) {
    extra.backend = backend;
    extra.agent_name = agent_name || name;
    if (agent_id) extra.agent_id = agent_id;
    if (cli_path) extra.cli_path = cli_path;
  }

  if (session_mode) extra.session_mode = session_mode;
  if (current_model_id) extra.current_model_id = current_model_id;

  return {
    type,
    model,
    name,
    preset_id: is_preset ? preset_id : undefined,
    extra,
  };
}
