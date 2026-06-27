/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import { isConversationProcessing } from '@/renderer/pages/conversation/utils/conversationRuntime';
import { useRunLive } from '@/renderer/pages/orchestrator/useRunLive';
import { useMemo } from 'react';

/**
 * Coarse orchestration phase the lead conversation is in, used by the status
 * strip to pick an icon + label + (optionally) a primary CTA.
 *
 * The three run-backed phases (`awaiting` / `running` / `completed`) are the
 * load-bearing ones — they come cleanly from {@link useRunLive} and MUST be
 * distinguished. The two no-run phases (`planning` / `idle`) are derived from a
 * best-effort read of the lead conversation's turn state and degrade gracefully
 * (see {@link deriveNoRunPhase}).
 */
export type OrchestrationPhase =
  | 'planning' // 主管规划中（首回合处理中，尚无 run）
  | 'idle' // 主管直接作答 / 未拆分（首回合结束，无 run）
  | 'error' // 未配置可用模型 / 主管未能运行
  | 'awaiting' // run awaiting_plan_approval（待批准）
  | 'running' // run running（协作中）
  | 'paused' // run paused
  | 'completed' // run completed
  | 'failed' // run failed
  | 'cancelled'; // run cancelled

export interface OrchestrationStatus {
  phase: OrchestrationPhase;
  /** The run id, when a run exists (drives the DAG tab + auto-reveal guard). */
  runId?: string;
  /** Completed task count (run-backed phases only). */
  done: number;
  /** Total task count (run-backed phases only). */
  total: number;
}

/** Statuses that count as "done" for the aggregate progress (mirrors DagCanvas). */
const DONE_STATUSES = new Set(['done', 'completed', 'skipped', 'cancelled']);

/** Map a backend run.status string onto our phase enum. */
function mapRunStatus(status: string): OrchestrationPhase {
  switch (status) {
    case 'awaiting_plan_approval':
      return 'awaiting';
    case 'running':
      return 'running';
    case 'planning':
      return 'planning';
    case 'paused':
      return 'paused';
    case 'completed':
      return 'completed';
    case 'failed':
      return 'failed';
    case 'cancelled':
      return 'cancelled';
    default:
      // Unknown / future status: treat as running so the strip stays visible
      // and points at the DAG rather than silently disappearing.
      return 'running';
  }
}

/**
 * Best-effort no-run phase. We only have the conversation record's runtime
 * snapshot (`runtime.is_processing`) to lean on — there is no clean per-turn
 * "last turn errored" flag on the conversation, so the `error` phase is left to
 * the run-backed `failed` branch and any future explicit signal. Degrade rules:
 *   - `is_processing` true → 主管规划中 (the lead's first turn is running, no run yet).
 *   - otherwise → 未拆分 (lead answered directly / hasn't decomposed the goal).
 */
function deriveNoRunPhase(conversation: TChatConversation): OrchestrationPhase {
  return isConversationProcessing(conversation) ? 'planning' : 'idle';
}

type OrchestratorExtra = {
  orchestrator_role?: string;
  orchestrator_run_id?: string;
};

/**
 * Derive the lead conversation's orchestration status for the status strip.
 *
 * Returns `null` for any conversation that is not an orchestration lead
 * (`extra.orchestrator_role !== 'lead'`), so callers render nothing. For a lead:
 *   - `extra.orchestrator_run_id` present → live run state via {@link useRunLive}
 *     (status → phase, plus done/total task counts).
 *   - no run id yet → a degraded turn-state read (`planning` while processing,
 *     else `idle`).
 *
 * `useRunLive` is always invoked (with `undefined` when there is no run) to keep
 * hook order stable across renders, per the rules of hooks.
 */
export function useOrchestrationStatus(conversation: TChatConversation | undefined): OrchestrationStatus | null {
  const extra = conversation?.extra as OrchestratorExtra | undefined;
  const isLead = extra?.orchestrator_role === 'lead';
  const runId = isLead ? extra?.orchestrator_run_id : undefined;

  // Always call the hook (undefined runId → no subscription) so the hook count
  // is invariant whether or not a run exists.
  const { detail } = useRunLive(runId);

  return useMemo<OrchestrationStatus | null>(() => {
    if (!conversation || !isLead) return null;

    if (runId && detail?.run) {
      const tasks = detail.tasks ?? [];
      const done = tasks.filter((task) => DONE_STATUSES.has(task.status)).length;
      return {
        phase: mapRunStatus(detail.run.status),
        runId,
        done,
        total: tasks.length,
      };
    }

    // Run id present but detail not loaded yet: hold a neutral planning phase so
    // the strip stays visible (and the DAG auto-reveal still fires) instead of
    // flickering to the no-run "未拆分" copy.
    if (runId) {
      return { phase: 'planning', runId, done: 0, total: 0 };
    }

    return { phase: deriveNoRunPhase(conversation), done: 0, total: 0 };
  }, [conversation, isLead, runId, detail]);
}
