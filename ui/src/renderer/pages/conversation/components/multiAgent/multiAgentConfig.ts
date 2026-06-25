/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Session-level "multi-agent collaboration" configuration (spec §6).
 *
 * This is the persisted config layer only — it lives on a conversation's
 * `extra.multi_agent`. Turning the feature on persists `enabled = true`; the
 * runtime team-ensure (lead = current conversation, building subagents) is a
 * separate concern handled at send/turn time (Task 14). Nothing here calls
 * `team.create`.
 */

/** How subagents are decided for a turn.
 *  - `auto`   — the leader decides the count / shape / models by task complexity.
 *  - `manual` — the user pre-declares the subagent roster (`manual_agents`). */
export type TMultiAgentMode = 'auto' | 'manual';

/** One manually-declared subagent: which execution backend + which model.
 *  `backend` mirrors `TeamAgentOption.backend` (claude / gemini / nomi / …);
 *  `model` is the resolved model ID the runtime will send to the team API. */
export type TMultiAgentManualAgent = {
  backend: string;
  model: string;
  /** Optional display label; falls back to the backend name in the UI. */
  name?: string;
};

export type TMultiAgentConfig = {
  enabled: boolean;
  mode: TMultiAgentMode;
  /** Only meaningful in `manual` mode; ignored when `mode === 'auto'`. */
  manual_agents?: TMultiAgentManualAgent[];
};

/** Default config used to seed the editable form before a save exists. */
export function defaultMultiAgentConfig(): TMultiAgentConfig {
  return { enabled: false, mode: 'auto', manual_agents: [] };
}

const isMode = (v: unknown): v is TMultiAgentMode => v === 'auto' || v === 'manual';

/** Coerce one raw roster entry into a well-formed manual agent, or drop it.
 *  A manual agent is only useful with a non-empty backend; model defaults to
 *  an empty string (the form / runtime resolves a concrete value later). */
function normalizeManualAgent(raw: unknown): TMultiAgentManualAgent | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const backend = typeof r.backend === 'string' ? r.backend.trim() : '';
  if (!backend) return null;
  const model = typeof r.model === 'string' ? r.model : '';
  const agent: TMultiAgentManualAgent = { backend, model };
  if (typeof r.name === 'string' && r.name.trim().length > 0) agent.name = r.name;
  return agent;
}

/**
 * Defensively read a persisted `extra.multi_agent` blob into a well-formed
 * config. Tolerates `undefined` / partial / legacy shapes — anything unparsable
 * falls back to the default. Never throws; the conversation header must always
 * be able to render a sane control state.
 */
export function normalizeMultiAgentConfig(raw: unknown): TMultiAgentConfig {
  const base = defaultMultiAgentConfig();
  if (!raw || typeof raw !== 'object') return base;
  const r = raw as Record<string, unknown>;

  const enabled = r.enabled === true;
  const mode: TMultiAgentMode = isMode(r.mode) ? r.mode : base.mode;
  const manual_agents = Array.isArray(r.manual_agents)
    ? (r.manual_agents.map(normalizeManualAgent).filter(Boolean) as TMultiAgentManualAgent[])
    : [];

  return { enabled, mode, manual_agents };
}

/**
 * A manual-mode config is only "ready to enable" once every declared subagent
 * has both a backend and a resolved model. The control uses this to gate the
 * enable toggle (so the runtime never receives a half-filled roster) and to
 * decide whether the manual roster is even worth persisting.
 */
export function isMultiAgentConfigReady(cfg: TMultiAgentConfig): boolean {
  if (cfg.mode === 'auto') return true;
  const roster = cfg.manual_agents ?? [];
  if (roster.length === 0) return false;
  return roster.every((a) => a.backend.trim().length > 0 && a.model.trim().length > 0);
}
