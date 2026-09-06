/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ICreateConversationParams } from '@/common/adapter/ipcBridge';
import type { TProviderWithModel } from '@/common/config/storage';

export type BuildAgentConversationInput = {
  backend: string;
  name: string;
  agent_id?: string;
  agent_name?: string;
  workspace: string;
  model: TProviderWithModel;
  cli_path?: string;
  custom_workspace?: boolean;
  current_model_id?: string;
  extra?: Partial<ICreateConversationParams['extra']>;
};

export function buildAgentConversationParams(input: BuildAgentConversationInput): ICreateConversationParams {
  const {
    backend,
    name,
    agent_id,
    agent_name,
    workspace,
    model,
    cli_path,
    custom_workspace = true,
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

  extra.backend = backend;
  extra.agent_name = agent_name || name;
  if (agent_id) extra.agent_id = agent_id;
  if (cli_path) extra.cli_path = cli_path;

  if (current_model_id) extra.current_model_id = current_model_id;

  return {
    type,
    model,
    name,
    extra,
  };
}
