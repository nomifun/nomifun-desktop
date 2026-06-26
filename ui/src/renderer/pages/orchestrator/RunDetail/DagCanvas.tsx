/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ReactFlow, Background, BackgroundVariant, Controls, MiniMap, type Edge } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import './dag-canvas.css';
import { Branch } from '@icon-park/react';
import { Spin } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { TRunTask } from '@/common/types/orchestrator/orchestratorTypes';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { useRunLive } from '../useRunLive';
import { layoutDag } from './layoutDag';
import RunDetailHeader from './RunDetailHeader';
import TaskNode, { taskStatusMeta, type TaskFlowNode } from './nodes/TaskNode';

/** Stable nodeTypes ref so react-flow doesn't warn about a new object each render. */
const NODE_TYPES = { task: TaskNode } as const;

/** Statuses that count as "done" for the aggregate progress pill. */
const DONE_STATUSES = new Set(['done', 'completed', 'skipped', 'cancelled']);

interface DagCanvasProps {
  runId: string;
  onBack: () => void;
  onOpenTask: (task: TRunTask) => void;
}

/**
 * DagCanvas — the visual centerpiece of 「智能编排」. Renders a run's task DAG as
 * an interactive react-flow graph: each task is a custom {@link TaskNode}, each
 * `blocker → blocked` dependency is an edge (animated while the downstream task
 * runs). Live-updates via {@link useRunLive}; clicking a node opens the worker
 * transcript panel (Task 5) through `onOpenTask`.
 *
 * Positions prefer the task's persisted `graph_x/graph_y` and otherwise fall
 * back to a topological auto-layout ({@link layoutDag}). react-flow's JS-side
 * colors (MiniMap mask, Background dots) can't read CSS vars, so we mirror the
 * `data-theme` attribute into `colorMode` + resolved colors via a MutationObserver
 * (template: MermaidBlock).
 */
const DagCanvas: React.FC<DagCanvasProps> = ({ runId, onBack, onOpenTask }) => {
  const { t } = useTranslation();
  const { detail, loading } = useRunLive(runId);
  const [message, ctx] = useArcoMessage();
  const [cancelling, setCancelling] = useState(false);

  // Mirror the global data-theme attribute (light/dark) for react-flow internals
  // whose colors are JS props (MiniMap mask, Background dots) and cannot read CSS
  // vars. Same observer pattern as MermaidBlock.
  const [theme, setTheme] = useState<'light' | 'dark'>(() =>
    (document.documentElement.getAttribute('data-theme') as 'light' | 'dark') || 'light'
  );
  useEffect(() => {
    const update = () => {
      setTheme((document.documentElement.getAttribute('data-theme') as 'light' | 'dark') || 'light');
    };
    const observer = new MutationObserver(update);
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return () => observer.disconnect();
  }, []);

  // Resolved JS-side colors for react-flow internals (theme-matched, no CSS vars).
  const flowColors = useMemo(
    () =>
      theme === 'dark'
        ? { dots: '#333333', minimapMask: 'rgba(0,0,0,0.55)', minimapBg: '#1a1a1a', minimapStroke: '#404040' }
        : { dots: '#d1d5e5', minimapMask: 'rgba(255,255,255,0.6)', minimapBg: '#f9fafb', minimapStroke: '#e5e6eb' },
    [theme]
  );

  // task_id → assignment member id (for the node chip).
  const memberByTask = useMemo(() => {
    const map = new Map<string, string>();
    for (const a of detail?.assignments ?? []) map.set(a.task_id, a.member_id);
    return map;
  }, [detail?.assignments]);

  const nodes = useMemo<TaskFlowNode[]>(() => {
    const tasks = detail?.tasks ?? [];
    const deps = detail?.deps ?? [];
    if (tasks.length === 0) return [];
    const fallback = layoutDag(tasks, deps);
    return tasks.map((task) => {
      const pos =
        task.graph_x != null && task.graph_y != null
          ? { x: task.graph_x, y: task.graph_y }
          : (fallback[task.id] ?? { x: 0, y: 0 });
      const memberId = memberByTask.get(task.id);
      return {
        id: task.id,
        type: 'task',
        position: pos,
        data: {
          title: task.title || t('orchestrator.run.detail.untitledTask'),
          status: task.status,
          statusLabel: t(`orchestrator.run.task.status.${task.status}`, {
            defaultValue: t('orchestrator.run.status.unknown'),
          }),
          memberId,
          chipLabel: memberId ? t('orchestrator.run.detail.assigned') : undefined,
          attempt: task.attempt,
          onOpen: () => onOpenTask(task),
        },
      };
    });
  }, [detail?.tasks, detail?.deps, memberByTask, onOpenTask, t]);

  const edges = useMemo<Edge[]>(() => {
    const tasks = detail?.tasks ?? [];
    const deps = detail?.deps ?? [];
    const statusById = new Map(tasks.map((task) => [task.id, task.status]));
    return deps.map((dep) => {
      const downstreamRunning = statusById.get(dep.blocked_task_id) === 'running';
      return {
        id: `${dep.blocker_task_id}->${dep.blocked_task_id}`,
        source: dep.blocker_task_id,
        target: dep.blocked_task_id,
        animated: downstreamRunning,
        style: {
          stroke: downstreamRunning ? 'rgb(var(--primary-6))' : 'var(--border-base)',
          strokeWidth: downstreamRunning ? 2 : 1.5,
        },
      };
    });
  }, [detail?.tasks, detail?.deps]);

  const { done, total } = useMemo(() => {
    const tasks = detail?.tasks ?? [];
    return {
      done: tasks.filter((task) => DONE_STATUSES.has(task.status)).length,
      total: tasks.length,
    };
  }, [detail?.tasks]);

  const handleCancel = async () => {
    setCancelling(true);
    try {
      await ipcBridge.orchestrator.runs.cancel.invoke({ id: runId });
      message.success(t('orchestrator.run.detail.cancelOk'));
    } catch (e) {
      message.error(t('orchestrator.run.detail.cancelError', { error: String(e) }));
    } finally {
      setCancelling(false);
    }
  };

  // First load with no detail yet.
  if (loading && !detail) {
    return (
      <div className='flex size-full min-h-0 flex-col'>
        <div className='flex flex-1 items-center justify-center'>
          <Spin />
        </div>
      </div>
    );
  }

  if (!detail) {
    return (
      <div className='flex size-full min-h-0 flex-col items-center justify-center gap-12px px-24px text-center'>
        <span className='flex size-48px items-center justify-center rd-14px bg-fill-2 text-t-tertiary'>
          <Branch theme='outline' size='24' strokeWidth={3} />
        </span>
        <div className='text-15px font-600 text-t-primary'>{t('orchestrator.run.detail.loadError')}</div>
      </div>
    );
  }

  const noTasks = detail.tasks.length === 0;

  return (
    <div className='size-full min-h-0 flex flex-col'>
      {ctx}
      <RunDetailHeader
        run={detail.run}
        done={done}
        total={total}
        onBack={onBack}
        onCancel={() => void handleCancel()}
        cancelling={cancelling}
      />

      <div className='flex-1 min-h-0'>
        {noTasks ? (
          <div className='flex size-full flex-col items-center justify-center gap-12px px-24px text-center'>
            <span className='nomi-dag-pulse flex size-52px items-center justify-center rd-16px bg-fill-2 text-primary-6'>
              <Branch theme='outline' size='26' strokeWidth={3} />
            </span>
            <div className='text-15px font-600 text-t-primary'>{t('orchestrator.run.detail.planningTitle')}</div>
            <div className='max-w-320px text-12px leading-18px text-t-tertiary'>
              {t('orchestrator.run.detail.planningDesc')}
            </div>
          </div>
        ) : (
          <ReactFlow
            className='nomi-dag-flow'
            nodes={nodes}
            edges={edges}
            nodeTypes={NODE_TYPES}
            colorMode={theme}
            fitView
            fitViewOptions={{ padding: 0.25, maxZoom: 1.1 }}
            minZoom={0.2}
            maxZoom={1.8}
            proOptions={{ hideAttribution: true }}
            nodesConnectable={false}
            nodesDraggable
            elementsSelectable
          >
            <Background variant={BackgroundVariant.Dots} gap={20} size={1.4} color={flowColors.dots} />
            <Controls showInteractive={false} />
            <MiniMap
              pannable
              zoomable
              maskColor={flowColors.minimapMask}
              style={{ background: flowColors.minimapBg, border: `1px solid ${flowColors.minimapStroke}` }}
              nodeColor={(n) => taskStatusMeta(String((n.data as { status?: string }).status ?? '')).color}
              nodeStrokeWidth={2}
            />
          </ReactFlow>
        )}
      </div>
    </div>
  );
};

export default DagCanvas;
