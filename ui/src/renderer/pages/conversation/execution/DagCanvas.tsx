import { Spin } from '@arco-design/web-react';
import { Branch } from '@icon-park/react';
import { Background, BackgroundVariant, Controls, MiniMap, ReactFlow, type Edge, type ReactFlowInstance } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type {
  TAgentExecutionDetail,
  TExecutionAttempt,
  TExecutionParticipant,
  TExecutionStep,
} from '@/common/types/agentExecution/agentExecutionTypes';
import { latestAttemptForStep } from '@/common/types/agentExecution/agentExecutionTypes';
import ParticipantProfilePanel from './ParticipantProfilePanel';
import { layoutExecutionDag } from './layoutExecutionDag';
import { participantLogo, participantShortLabel } from './participantLabel';
import StepNode, { type JudgeWinner, type LoopState, normalizeStepKind, type StepFlowNode, type VerifyVerdict } from './nodes/StepNode';
import type { LeadThinkingState } from './useLeadThinking';
import './dag-canvas.css';

const NODE_TYPES = { step: StepNode } as const;
const FIT_VIEW_OPTIONS = { padding: 0.12, maxZoom: 1.6 } as const;
const NODE_WIDTH = 220;
const NODE_HEIGHT = 96;

const VERDICT_RE = /^VERDICT:\s+(PASS|FAIL)\s+\((\d+)\/(\d+)/;
const WINNER_RE = /^WINNER:\s+(?:candidate\s+(\d+)|none)/;
const WINNER_AGGREGATION_RE = /aggregate=(mean|borda)/;
const WINNER_JUDGES_RE = /judges=(\d+\/\d+)/;
const LOOP_RE = /^LOOP:\s+(DONE|FAILED)\s+\(reason=([a-z_]+),\s*iterations=(\d+),\s*max_iter=(\d+)\)/;

function parseVerifyVerdict(summary?: string): VerifyVerdict {
  const match = summary ? VERDICT_RE.exec(summary.trim()) : null;
  return match ? { pass: match[1] === 'PASS', tally: `${match[2]}/${match[3]}` } : { pass: null, tally: null };
}

function parseJudgeWinner(summary?: string): JudgeWinner {
  const match = summary ? WINNER_RE.exec(summary.trim()) : null;
  if (!match) return { winner: null, aggregate: null, judges: null };
  const aggregation = WINNER_AGGREGATION_RE.exec(summary ?? '');
  const judges = WINNER_JUDGES_RE.exec(summary ?? '');
  return {
    winner: match[1] == null ? null : Number(match[1]),
    aggregate: aggregation ? (aggregation[1] as 'mean' | 'borda') : null,
    judges: judges?.[1] ?? null,
  };
}

function parseLoopState(summary?: string): LoopState {
  const match = summary ? LOOP_RE.exec(summary.trim()) : null;
  return match
    ? {
        state: match[1] === 'DONE' ? 'done' : 'failed',
        reason: match[2],
        iterations: Number(match[3]),
        maxIter: Number(match[4]),
      }
    : { state: null, reason: null, iterations: null, maxIter: null };
}

function hueForGroup(label: string): number {
  let hash = 0;
  for (let index = 0; index < label.length; index += 1) {
    hash = (hash * 31 + label.charCodeAt(index)) % 360;
  }
  return hash;
}

const MINI_MAP_COLORS: Record<'light' | 'dark', Record<string, string>> = {
  light: {
    running: '#2f6bff',
    completed: '#16a34a',
    failed: '#dc2626',
    waiting_input: '#d97706',
    skipped: '#94a3b8',
    cancelled: '#94a3b8',
    pending: '#b4bccb',
  },
  dark: {
    running: '#5b8bff',
    completed: '#22c55e',
    failed: '#f04438',
    waiting_input: '#f59e0b',
    skipped: '#64748b',
    cancelled: '#64748b',
    pending: '#5a6273',
  },
};

export interface OpenStepPayload {
  /** Stable UI identity across immutable replacements of the same selected step. */
  projectionKey?: string;
  step: TExecutionStep;
  participant?: TExecutionParticipant;
  participants: TExecutionParticipant[];
  attempt?: TExecutionAttempt;
  executionId: string;
  refetch: () => Promise<void>;
}

interface DagCanvasProps {
  executionId: string;
  detail: TAgentExecutionDetail | null;
  loading: boolean;
  refetch: () => Promise<void>;
  onOpenStep: (payload: OpenStepPayload) => void;
  leadThinking?: LeadThinkingState;
  activeStepId?: string | null;
}

const DagCanvas: React.FC<DagCanvasProps> = ({ executionId, detail, loading, refetch, onOpenStep, leadThinking, activeStepId }) => {
  const { t, i18n } = useTranslation();
  const flowInstance = useRef<ReactFlowInstance<StepFlowNode, Edge> | null>(null);
  const wrapper = useRef<HTMLDivElement | null>(null);
  const [theme, setTheme] = useState<'light' | 'dark'>(() =>
    document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'light',
  );

  useEffect(() => {
    const updateTheme = () => setTheme(document.documentElement.getAttribute('data-theme') === 'dark' ? 'dark' : 'light');
    const observer = new MutationObserver(updateTheme);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['data-theme'],
    });
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const element = wrapper.current;
    if (!element) return;
    const observer = new ResizeObserver(() => {
      requestAnimationFrame(() => flowInstance.current?.fitView(FIT_VIEW_OPTIONS));
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const participantById = useMemo(
    () => new Map((detail?.participants ?? []).map((participant) => [participant.id, participant])),
    [detail?.participants],
  );

  const activeSteps = useMemo(() => (detail?.steps ?? []).filter((step) => step.superseded_in_revision == null), [detail?.steps]);
  const activeDependencies = useMemo(
    () => (detail?.dependencies ?? []).filter((dependency) => dependency.superseded_in_revision == null),
    [detail?.dependencies],
  );

  const latestAttemptByStep = useMemo(
    () => new Map(activeSteps.map((step) => [step.id, latestAttemptForStep(detail?.attempts ?? [], step.id)])),
    [activeSteps, detail?.attempts],
  );

  const nodes = useMemo<StepFlowNode[]>(() => {
    const steps = activeSteps;
    const dependencies = activeDependencies;
    const fallbackPositions = layoutExecutionDag(steps, dependencies);

    return steps.map((step, index) => {
      const participant = step.assigned_participant_id ? participantById.get(step.assigned_participant_id) : undefined;
      const attempt = latestAttemptByStep.get(step.id);
      const displayKind = step.kind === 'agent' && step.agent_mode === 'synthesis' ? 'synthesis' : step.kind;
      const normalizedKind = normalizeStepKind(displayKind);
      const groupLabel = step.fanout_group?.trim() || undefined;
      const outputSummary = attempt?.output_summary ?? undefined;

      return {
        id: step.id,
        type: 'step',
        selected: activeStepId === step.id,
        position:
          step.graph_x != null && step.graph_y != null
            ? { x: step.graph_x, y: step.graph_y }
            : (fallbackPositions[step.id] ?? { x: 0, y: 0 }),
        initialWidth: NODE_WIDTH,
        initialHeight: NODE_HEIGHT,
        data: {
          title: step.title || t('agentExecution.step.untitled', { defaultValue: '未命名任务' }),
          status: step.status,
          statusLabel: t(`agentExecution.status.step.${step.status}`, {
            defaultValue: step.status,
          }),
          kind: displayKind,
          enterIndex: Math.min(index, 12),
          synthesisLabel: normalizedKind === 'synthesis' ? t('agentExecution.kind.synthesis', { defaultValue: '汇总' }) : undefined,
          verifyLabel: normalizedKind === 'verify' ? t('agentExecution.kind.verify', { defaultValue: '验证' }) : undefined,
          judgeLabel: normalizedKind === 'judge' ? t('agentExecution.kind.judge', { defaultValue: '评审' }) : undefined,
          loopLabel: normalizedKind === 'loop' ? t('agentExecution.kind.loop', { defaultValue: '循环' }) : undefined,
          verifyVerdict: normalizedKind === 'verify' ? parseVerifyVerdict(outputSummary) : undefined,
          verifyVerdictLabels: {
            pass: t('agentExecution.verdict.pass', { defaultValue: '通过' }),
            fail: t('agentExecution.verdict.fail', { defaultValue: '未通过' }),
            pending: t('agentExecution.verdict.pending', {
              defaultValue: '验证中',
            }),
          },
          judgeWinner: normalizedKind === 'judge' ? parseJudgeWinner(outputSummary) : undefined,
          judgeWinnerLabels: {
            winner: t('agentExecution.judge.winner', { defaultValue: '胜出' }),
            none: t('agentExecution.judge.none', { defaultValue: '暂无结果' }),
            pending: t('agentExecution.judge.pending', {
              defaultValue: '评审中',
            }),
          },
          loopState: normalizedKind === 'loop' ? parseLoopState(outputSummary) : undefined,
          loopStateLabels: {
            done: t('agentExecution.loop.done', { defaultValue: '完成' }),
            failed: t('agentExecution.loop.failed', { defaultValue: '失败' }),
            iterating: t('agentExecution.loop.iterating', {
              defaultValue: '迭代中',
            }),
          },
          groupLabel,
          groupHue: groupLabel ? hueForGroup(groupLabel) : undefined,
          groupChipLabel: groupLabel
            ? t('agentExecution.kind.parallelGroup', {
                defaultValue: '并行：{{label}}',
                label: groupLabel,
              })
            : undefined,
          participantId: participant?.id,
          chipLabel: participantShortLabel(participant) ?? undefined,
          participantLogo: participantLogo(participant),
          locked: step.assignment_locked,
          attempt: attempt?.attempt_no ?? 0,
          tokens: attempt?.tokens ?? undefined,
          tokensLabel: t('agentExecution.step.tokens', {
            defaultValue: 'tokens',
          }),
          pendingQuestion: attempt?.question ?? undefined,
          questionLabel: t('agentExecution.step.waitingInput', {
            defaultValue: '待回答',
          }),
          onOpen: () =>
            onOpenStep({
              step,
              participant,
              participants: detail?.participants ?? [],
              attempt,
              executionId,
              refetch,
            }),
        },
      };
    });
  }, [
    activeStepId,
    activeDependencies,
    activeSteps,
    detail?.participants,
    executionId,
    i18n.language,
    latestAttemptByStep,
    onOpenStep,
    participantById,
    refetch,
    t,
  ]);

  const edges = useMemo<Edge[]>(() => {
    const statusById = new Map(activeSteps.map((step) => [step.id, step.status]));
    return activeDependencies.map((dependency) => {
      const animated = statusById.get(dependency.blocked_step_id) === 'running';
      return {
        id: `${dependency.blocker_step_id}->${dependency.blocked_step_id}`,
        source: dependency.blocker_step_id,
        target: dependency.blocked_step_id,
        animated,
        className: animated ? 'nomi-dag-edge-live' : undefined,
        style: {
          stroke: animated ? 'rgb(var(--primary-6))' : 'var(--border-base)',
          strokeWidth: animated ? 2 : 1.5,
        },
      };
    });
  }, [activeDependencies, activeSteps]);

  if (loading && !detail) {
    return <Spin className='m-auto' />;
  }
  if (!detail) {
    return (
      <div className='flex size-full flex-col items-center justify-center gap-12px text-t-tertiary'>
        <Branch theme='outline' size='24' />
        {t('agentExecution.detail.loadError', {
          defaultValue: '协作进度加载失败',
        })}
      </div>
    );
  }

  return (
    <div className='size-full min-h-0 flex flex-col'>
      {(detail.execution.status === 'completed' || detail.execution.status === 'completed_with_failures') && (
        <ParticipantProfilePanel detail={detail} />
      )}
      <div ref={wrapper} className='flex-1 min-h-0'>
        {activeSteps.length === 0 ? (
          <div className='flex size-full flex-col items-center justify-center gap-10px px-24px text-center'>
            <Branch className='nomi-dag-pulse text-primary-6' theme='outline' size='26' />
            <strong>
              {t('agentExecution.thinking.title', {
                defaultValue: '正在准备协作计划',
              })}
            </strong>
            {leadThinking?.active && leadThinking.reasoning && (
              <span className='max-w-340px text-11px text-t-tertiary line-clamp-3'>{leadThinking.reasoning.slice(-160)}</span>
            )}
          </div>
        ) : (
          <ReactFlow
            className='nomi-dag-flow'
            onInit={(instance) => {
              flowInstance.current = instance;
            }}
            nodes={nodes}
            edges={edges}
            nodeTypes={NODE_TYPES}
            colorMode={theme}
            fitView
            fitViewOptions={FIT_VIEW_OPTIONS}
            minZoom={0.2}
            maxZoom={1.8}
            proOptions={{ hideAttribution: true }}
            nodesConnectable={false}
            nodesDraggable
            elementsSelectable={false}
          >
            <Background variant={BackgroundVariant.Dots} gap={22} size={1.2} color={theme === 'dark' ? '#333333' : '#d1d5e5'} />
            <Controls showInteractive={false} />
            <MiniMap
              pannable
              zoomable
              position='top-right'
              maskColor={theme === 'dark' ? 'rgba(0,0,0,.55)' : 'rgba(255,255,255,.6)'}
              nodeColor={(node) => {
                const status = String((node.data as { status?: string }).status ?? 'pending');
                return MINI_MAP_COLORS[theme][status] ?? MINI_MAP_COLORS[theme].pending;
              }}
              nodeStrokeWidth={2}
            />
          </ReactFlow>
        )}
      </div>
    </div>
  );
};

export default DagCanvas;
