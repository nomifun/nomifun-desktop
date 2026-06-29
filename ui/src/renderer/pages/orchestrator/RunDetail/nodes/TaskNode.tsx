/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react';
import { Branch, Lock, Merge } from '@icon-park/react';

/** Task status → theme-var color + a slow-pulse hint for the running state. */
export interface TaskStatusMeta {
  /** CSS color expression (theme var). */
  color: string;
  /** Whether the status dot should pulse (running). */
  pulse: boolean;
}

/**
 * Map a task status string to its on-brand color. Statuses come straight off
 * the wire (`TRunTask.status`), so unknown values fall back to a muted tone.
 *
 * pending → tertiary text · running → brand primary (pulsing) · done → success
 * · failed → danger · needs_review → warning · skipped → muted.
 */
export function taskStatusMeta(status: string): TaskStatusMeta {
  switch (status) {
    case 'running':
      return { color: 'rgb(var(--primary-6))', pulse: true };
    case 'done':
    case 'completed':
      return { color: 'var(--success)', pulse: false };
    case 'failed':
    case 'error':
      return { color: 'var(--danger)', pulse: false };
    case 'needs_review':
    case 'blocked':
      return { color: 'var(--warning)', pulse: false };
    case 'skipped':
    case 'cancelled':
      return { color: 'var(--text-disabled)', pulse: false };
    case 'pending':
    default:
      return { color: 'var(--bg-6)', pulse: false };
  }
}

/** The synthesis task kind — a node that merges its upstream tasks' outputs into
 * a final result. Every other (or unknown) `kind` renders as a plain agent node
 * with zero visual change, so the common case is untouched. */
export const TASK_KIND_SYNTHESIS = 'synthesis';

/**
 * Normalize a raw `TRunTask.kind` defensively. The wire value defaults to
 * `'agent'` on the backend, but legacy / malformed values must never crash the
 * canvas — anything we don't recognize collapses to `'agent'` (no badge).
 */
export function normalizeTaskKind(kind: string | null | undefined): 'agent' | 'synthesis' {
  return kind === TASK_KIND_SYNTHESIS ? 'synthesis' : 'agent';
}

/** Brand-tinted accent for the synthesis badge — intentionally distinct from the
 * status palette (success/danger/warning/primary) so a synthesis node reads as a
 * structural role, not a status. Defined in every theme preset. */
const SYNTH_ACCENT = 'var(--brand)';

/** The data payload DagCanvas attaches to each task node. */
export interface TaskNodeData extends Record<string, unknown> {
  title: string;
  status: string;
  statusLabel: string;
  /** Raw task kind off the wire (`'agent'` default | `'synthesis'`). Rendered
   * defensively via {@link normalizeTaskKind}; unknown → agent (no badge). */
  kind?: string;
  /** Localized "synthesis" label for the kind badge (computed in DagCanvas so the
   * node stays free of i18n wiring). */
  synthesisLabel?: string;
  /** Fan-out group label parsed from `pattern_config` (`{"group":"<label>"}`).
   * Present only for sibling tasks the planner fanned out in parallel. */
  groupLabel?: string;
  /** Per-group hue (HSL degrees) so every sibling in a fan-out group shares one
   * tint — a calm, deterministic color derived from the group label. */
  groupHue?: number;
  /** Localized "fan-out: {{label}}" text for the group chip. */
  groupChipLabel?: string;
  /** Assigned fleet member id (raw uuid — used only for the chip tooltip). */
  memberId?: string;
  /** Friendly chip label resolved from the run's fleet snapshot:
   * agent id (+ model). Falls back to a localized "assigned" when the member
   * can't be resolved against `fleet_members`. */
  chipLabel?: string;
  /** Logo url for the assigned agent (resolved from agent_id), if any. */
  memberLogo?: string | null;
  attempt: number;
  /** Whether this assignment is locked (pinned against auto-routing). */
  locked?: boolean;
  /** Click handler — opens the task inspector / transcript panel. */
  onOpen: () => void;
}

/** Strongly-typed node alias so NodeProps narrows `data` for us. */
export type TaskFlowNode = Node<TaskNodeData, 'task'>;

/**
 * TaskNode — a custom react-flow node rendering one DAG task as an on-brand
 * card: status dot + left status border, title, an assignment chip, and a
 * retry-count badge. The whole card is a button that opens the task's
 * transcript panel. Theme variables only (no hardcoded hex); source/target
 * handles anchor the dependency edges.
 */
function TaskNodeImpl({ data, selected }: NodeProps<TaskFlowNode>) {
  const meta = taskStatusMeta(data.status);
  const isSynthesis = normalizeTaskKind(data.kind) === 'synthesis';
  // A fan-out group needs a label AND a resolved hue; either missing → no group
  // affordance (defensive against half-parsed config).
  const inGroup = data.groupLabel != null && data.groupHue != null;
  const groupColor = inGroup ? `hsl(${data.groupHue}, 62%, 55%)` : null;

  // Compose the card outline. Selection always wins; otherwise a fan-out sibling
  // gets a soft group-hued ring layered under the base drop shadow so the whole
  // group reads as one cohort without fighting the status left-border.
  const baseShadow = selected
    ? '0 0 0 3px color-mix(in srgb, rgb(var(--primary-6)) 22%, transparent), 0 6px 18px rgba(0,0,0,0.14)'
    : groupColor
      ? `0 0 0 2px color-mix(in srgb, ${groupColor} 38%, transparent), 0 2px 10px rgba(0,0,0,0.10)`
      : '0 2px 10px rgba(0,0,0,0.10)';

  return (
    <div
      role='button'
      tabIndex={0}
      aria-label={`${data.title} · ${data.statusLabel}`}
      onClick={data.onOpen}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          data.onOpen();
        }
      }}
      className='nomi-dag-node group flex w-220px cursor-pointer select-none flex-col gap-8px rd-12px px-14px py-12px transition-all duration-150 outline-none'
      style={{
        background: 'var(--bg-2)',
        border: `1px solid ${selected ? 'rgb(var(--primary-6))' : 'var(--border-base)'}`,
        borderLeft: `3px solid ${meta.color}`,
        boxShadow: baseShadow,
      }}
    >
      {/* Incoming-dependency anchor (top) */}
      <Handle
        type='target'
        position={Position.Top}
        isConnectable={false}
        style={{ width: 7, height: 7, background: 'var(--bg-5)', border: 'none' }}
      />

      {/* Title row: status dot + task title (+ synthesis badge, right-aligned) */}
      <div className='flex items-start gap-8px'>
        <span
          className={`mt-4px size-9px shrink-0 rd-full ${meta.pulse ? 'nomi-dag-pulse' : ''}`}
          style={{ background: meta.color, boxShadow: `0 0 0 3px color-mix(in srgb, ${meta.color} 20%, transparent)` }}
        />
        <span className='min-w-0 flex-1 text-13px font-600 leading-18px text-t-primary line-clamp-2'>
          {data.title}
        </span>
        {isSynthesis && (
          <span
            className='nomi-dag-kind-badge inline-flex shrink-0 items-center gap-3px rd-100px px-6px py-2px text-10px font-600 leading-none'
            style={{
              color: SYNTH_ACCENT,
              background: `color-mix(in srgb, ${SYNTH_ACCENT} 14%, transparent)`,
              border: `1px solid color-mix(in srgb, ${SYNTH_ACCENT} 32%, transparent)`,
            }}
            title={data.synthesisLabel}
          >
            <Merge theme='outline' size='10' strokeWidth={4} className='line-height-0' />
            {data.synthesisLabel}
          </span>
        )}
      </div>

      {/* Meta row: status label + assignment chip + retry badge + fan-out group chip */}
      <div className='flex flex-wrap items-center gap-6px'>
        <span className='text-11px font-500 leading-none' style={{ color: meta.color }}>
          {data.statusLabel}
        </span>
        {data.chipLabel && (
          <span
            className='inline-flex max-w-[150px] items-center gap-3px rd-100px px-6px py-2px text-10px leading-none text-t-secondary'
            style={{ background: 'var(--fill-0)', border: '1px solid var(--border-light)' }}
            title={data.memberId}
          >
            {data.memberLogo ? (
              <img src={data.memberLogo} alt='' className='size-10px shrink-0 object-contain' />
            ) : (
              <span
                className='size-5px shrink-0 rd-full'
                style={{ background: 'rgb(var(--primary-6))' }}
              />
            )}
            <span className='truncate'>{data.chipLabel}</span>
            {data.locked && (
              <Lock theme='outline' size='9' strokeWidth={4} className='shrink-0 text-t-tertiary' />
            )}
          </span>
        )}
        {data.attempt > 1 && (
          <span
            className='inline-flex items-center rd-100px px-6px py-2px text-10px leading-none'
            style={{ background: 'color-mix(in srgb, var(--warning) 16%, transparent)', color: 'var(--warning)' }}
          >
            ×{data.attempt}
          </span>
        )}
        {inGroup && groupColor && (
          <span
            className='inline-flex max-w-[150px] items-center gap-3px rd-100px px-6px py-2px text-10px font-500 leading-none'
            style={{
              color: groupColor,
              background: `color-mix(in srgb, ${groupColor} 14%, transparent)`,
              border: `1px solid color-mix(in srgb, ${groupColor} 30%, transparent)`,
            }}
            title={data.groupChipLabel}
          >
            <Branch theme='outline' size='10' strokeWidth={4} className='shrink-0 line-height-0' />
            <span className='truncate'>{data.groupChipLabel}</span>
          </span>
        )}
      </div>

      {/* Outgoing-dependency anchor (bottom) */}
      <Handle
        type='source'
        position={Position.Bottom}
        isConnectable={false}
        style={{ width: 7, height: 7, background: 'var(--bg-5)', border: 'none' }}
      />
    </div>
  );
}

export default React.memo(TaskNodeImpl);
