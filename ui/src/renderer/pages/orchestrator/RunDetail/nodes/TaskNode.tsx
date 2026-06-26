/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Handle, Position, type Node, type NodeProps } from '@xyflow/react';

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

/** The data payload DagCanvas attaches to each task node. */
export interface TaskNodeData extends Record<string, unknown> {
  title: string;
  status: string;
  statusLabel: string;
  /** Assigned fleet member id (P1b: raw id; friendly label is a follow-up). */
  memberId?: string;
  /** Localized "assigned" chip label (member id resolution is a follow-up). */
  chipLabel?: string;
  attempt: number;
  /** Click handler — opens the worker transcript panel (Task 5). */
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
        boxShadow: selected
          ? '0 0 0 3px color-mix(in srgb, rgb(var(--primary-6)) 22%, transparent), 0 6px 18px rgba(0,0,0,0.14)'
          : '0 2px 10px rgba(0,0,0,0.10)',
      }}
    >
      {/* Incoming-dependency anchor (top) */}
      <Handle
        type='target'
        position={Position.Top}
        isConnectable={false}
        style={{ width: 7, height: 7, background: 'var(--bg-5)', border: 'none' }}
      />

      {/* Title row: status dot + task title */}
      <div className='flex items-start gap-8px'>
        <span
          className={`mt-4px size-9px shrink-0 rd-full ${meta.pulse ? 'nomi-dag-pulse' : ''}`}
          style={{ background: meta.color, boxShadow: `0 0 0 3px color-mix(in srgb, ${meta.color} 20%, transparent)` }}
        />
        <span className='min-w-0 flex-1 text-13px font-600 leading-18px text-t-primary line-clamp-2'>
          {data.title}
        </span>
      </div>

      {/* Meta row: status label + assignment chip + retry badge */}
      <div className='flex flex-wrap items-center gap-6px'>
        <span className='text-11px font-500 leading-none' style={{ color: meta.color }}>
          {data.statusLabel}
        </span>
        {data.chipLabel && (
          <span
            className='inline-flex max-w-[120px] items-center gap-3px rd-100px px-6px py-2px text-10px leading-none text-t-secondary'
            style={{ background: 'var(--fill-0)', border: '1px solid var(--border-light)' }}
            title={data.memberId}
          >
            <span
              className='size-5px shrink-0 rd-full'
              style={{ background: 'rgb(var(--primary-6))' }}
            />
            <span className='truncate'>{data.chipLabel}</span>
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
