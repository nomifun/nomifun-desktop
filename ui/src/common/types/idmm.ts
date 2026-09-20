/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ConversationId, ProviderId } from './ids';

export type IdmmMode = 'off' | 'rule_only' | 'rule_plus_model';
export type IdmmScanScope = 'last_turn' | 'last_messages' | 'full_session';
export type IdmmRunState = 'off' | 'monitoring' | 'intervening' | 'degraded';
export type IdmmInterventionKind =
  | 'provider_failure'
  | 'stalled_turn'
  | 'option_decision'
  | 'open_question'
  | 'safety_halt';
export type IdmmInterventionStatus = 'pending' | 'succeeded' | 'failed' | 'halted';

export interface IIdmmConfig {
  mode: IdmmMode;
  scan_interval_secs: number;
  idle_timeout_secs: number;
  scan_scope: IdmmScanScope;
  max_context_messages: number;
  max_context_chars: number;
  recover_provider_failures: boolean;
  recover_stalled_turns: boolean;
  auto_select_options: boolean;
  prefer_recommended: boolean;
  max_retries: number;
  max_interventions_per_hour: number;
  min_interval_secs: number;
  bypass_model: {
    provider_id?: ProviderId | null;
    model?: string | null;
  };
}

export interface IIdmmIntervention {
  intervention_id: string;
  fingerprint: string;
  kind: IdmmInterventionKind;
  status: IdmmInterventionStatus;
  action: string;
  tier: IdmmMode;
  reason: string;
  attempt: number;
  created_at: number;
  updated_at: number;
  detail?: string | null;
}

export interface IIdmmState {
  agent_session_id: ConversationId;
  revision: number;
  config: IIdmmConfig;
  run_state: IdmmRunState;
  last_checked_at?: number | null;
  recent_interventions: IIdmmIntervention[];
}

export const createDefaultIdmmConfig = (): IIdmmConfig => ({
  mode: 'off',
  scan_interval_secs: 15,
  idle_timeout_secs: 90,
  scan_scope: 'last_messages',
  max_context_messages: 12,
  max_context_chars: 8000,
  recover_provider_failures: true,
  recover_stalled_turns: true,
  auto_select_options: true,
  prefer_recommended: true,
  max_retries: 3,
  max_interventions_per_hour: 20,
  min_interval_secs: 10,
  bypass_model: { provider_id: null, model: null },
});
