/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseMessageId, parseProviderId, type ConversationId, type ProviderId } from './ids';

/** Canonical explanation attached to an automatic reply or decision notice. */
export interface IdmmDecisionExplanation {
  intervention_id: string;
  source: 'rule' | 'bypass_model' | 'recovery';
  reason_code: string;
  rationale: string;
  model?: { provider_id: string; model: string };
  question?: { message_id: string; sequence: number; fingerprint: string };
}

export interface IdmmDecisionNotice {
  decision: IdmmDecisionExplanation;
  status: 'waiting_for_human' | 'failed';
  created_at: number;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value);
const boundedString = (value: unknown, maximum: number): value is string =>
  typeof value === 'string' && value.trim().length > 0 && value.length <= maximum;
const utf8Length = (value: string): number => new TextEncoder().encode(value).length;

/** Validate typed backend metadata; origin tags and message prose are never evidence. */
export const normalizeIdmmDecisionExplanation = (value: unknown): IdmmDecisionExplanation | undefined => {
  if (!isRecord(value)
    || typeof value.source !== 'string' || !['rule', 'bypass_model', 'recovery'].includes(value.source)
    || typeof value.reason_code !== 'string' || !/^[a-z0-9_]{1,80}$/.test(value.reason_code)
    || typeof value.rationale !== 'string' || !value.rationale || value.rationale.trim() !== value.rationale
    || /\p{Cc}/u.test(value.rationale) || Array.from(value.rationale).length > 40
    || utf8Length(value.rationale) > 160
    || (value.source === 'bypass_model') !== (value.model != null)) return undefined;
  let interventionId: string;
  let model: IdmmDecisionExplanation['model'];
  let question: IdmmDecisionExplanation['question'];
  try {
    interventionId = parseMessageId(value.intervention_id);
    if (value.model != null) {
      if (!isRecord(value.model) || !boundedString(value.model.model, 200)
        || value.model.model.trim() !== value.model.model || /\p{Cc}/u.test(value.model.model)
        || utf8Length(value.model.model) > 200) return undefined;
      model = { provider_id: parseProviderId(value.model.provider_id), model: value.model.model };
    }
    if (value.question != null) {
      if (!isRecord(value.question) || !Number.isSafeInteger(value.question.sequence)
        || (value.question.sequence as number) < 1 || typeof value.question.fingerprint !== 'string'
        || !/^[a-f0-9]{64}$/.test(value.question.fingerprint)) return undefined;
      question = {
        message_id: parseMessageId(value.question.message_id),
        sequence: value.question.sequence as number,
        fingerprint: value.question.fingerprint,
      };
    }
  } catch { return undefined; }
  if (value.source !== 'recovery' && !question) return undefined;
  return {
    intervention_id: interventionId,
    source: value.source as IdmmDecisionExplanation['source'],
    reason_code: value.reason_code,
    rationale: value.rationale,
    ...(model ? { model } : {}),
    ...(question ? { question } : {}),
  };
};

export const normalizeIdmmDecisionNotice = (value: unknown): IdmmDecisionNotice | undefined => {
  if (!isRecord(value) || typeof value.status !== 'string' || !['waiting_for_human', 'failed'].includes(value.status)
    || typeof value.created_at !== 'number' || !Number.isSafeInteger(value.created_at)
    || value.created_at <= 0 || value.created_at > 8.64e15) return undefined;
  const decision = normalizeIdmmDecisionExplanation(value.decision);
  return decision?.question ? { decision, status: value.status as IdmmDecisionNotice['status'], created_at: value.created_at } : undefined;
};

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
