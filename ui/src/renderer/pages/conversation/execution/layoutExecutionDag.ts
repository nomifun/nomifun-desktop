/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TExecutionStep, TExecutionStepDependency } from '@/common/types/agentExecution/agentExecutionTypes';

/** Horizontal step between sibling tasks within the same dependency layer. */
const COL_STEP = 260;
/** Vertical step between dependency layers (depth → row). */
const ROW_STEP = 140;

/**
 * Compute simple topological-layered positions for an execution task DAG.
 *
 * Each task's vertical layer is its **depth** = the longest path of `blocker →
 * blocked` edges leading into it (roots are depth 0). Tasks sharing a depth are
 * spread horizontally so siblings never overlap. The result is a pure
 * `taskId → {x,y}` map; callers prefer `task.graph_x/graph_y` when present and
 * fall back to this layout otherwise.
 *
 * The depth pass is a fixpoint relaxation capped at `tasks.length` iterations,
 * so a malformed plan with a dependency **cycle** can never spin forever — it
 * simply settles at whatever depths the cap allows (good enough for a render).
 */
export function layoutExecutionDag(
  steps: TExecutionStep[],
  dependencies: TExecutionStepDependency[],
): Record<string, { x: number; y: number }> {
  const positions: Record<string, { x: number; y: number }> = {};
  if (steps.length === 0) return positions;

  // Only consider dependencies whose endpoints are current tasks.
  const stepIds = new Set(steps.map((step) => step.id));
  const edges = dependencies.filter((dependency) => stepIds.has(dependency.blocker_step_id) && stepIds.has(dependency.blocked_step_id));

  // depth[id] = longest path (in edges) from any root to this task.
  const depth = new Map<string, number>();
  for (const step of steps) depth.set(step.id, 0);

  // Relax depths to a fixpoint: a task sits one layer below its deepest blocker.
  // Cap iterations at task count so a cycle can't loop forever.
  for (let iter = 0; iter < steps.length; iter++) {
    let changed = false;
    for (const e of edges) {
      const next = (depth.get(e.blocker_step_id) ?? 0) + 1;
      if (next > (depth.get(e.blocked_step_id) ?? 0)) {
        depth.set(e.blocked_step_id, next);
        changed = true;
      }
    }
    if (!changed) break;
  }

  // Bucket tasks by depth, preserving their incoming order for stable columns.
  const layers = new Map<number, string[]>();
  for (const step of steps) {
    const d = depth.get(step.id) ?? 0;
    const bucket = layers.get(d);
    if (bucket) bucket.push(step.id);
    else layers.set(d, [step.id]);
  }

  // Place each layer as a centered horizontal row so the graph reads top-down.
  for (const [d, ids] of layers) {
    const rowWidth = (ids.length - 1) * COL_STEP;
    const startX = -rowWidth / 2;
    ids.forEach((id, col) => {
      positions[id] = { x: startX + col * COL_STEP, y: d * ROW_STEP };
    });
  }

  return positions;
}
