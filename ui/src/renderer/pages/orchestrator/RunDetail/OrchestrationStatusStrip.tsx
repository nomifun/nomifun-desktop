/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TChatConversation } from '@/common/config/storage';
import {
  WORKSPACE_STATE_EVENT,
  dispatchWorkspaceSelectTabEvent,
  dispatchWorkspaceToggleEvent,
  type WorkspaceStateDetail,
} from '@/renderer/utils/workspace/workspaceEvents';
import { Caution, CheckOne, CloseOne, LoadingOne, Pause, Peoples, Right } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import type { OrchestrationPhase } from './useOrchestrationStatus';
import { useOrchestrationStatus } from './useOrchestrationStatus';

/** The DAG rail tab key (mirrors ChatSlider's extraTab key). */
const DAG_TAB_KEY = 'orchestrator-dag';

type PhaseVisual = {
  /** Accent color token used for the dot, icon, and the soft tinted background. */
  accent: string;
  icon: React.ReactNode;
  /** Whether the leading icon should spin (active phases). */
  spin?: boolean;
};

const ICON_PROPS = { theme: 'outline' as const, size: '14', strokeWidth: 3 };

/** Per-phase accent + icon. Colors are theme vars only (no hardcoded hues). */
function phaseVisual(phase: OrchestrationPhase): PhaseVisual {
  switch (phase) {
    case 'awaiting':
      return { accent: 'rgb(var(--primary-6))', icon: <CheckOne {...ICON_PROPS} /> };
    case 'running':
      return { accent: 'rgb(var(--primary-6))', icon: <Peoples {...ICON_PROPS} /> };
    case 'planning':
      return { accent: 'var(--warning)', icon: <LoadingOne {...ICON_PROPS} />, spin: true };
    case 'paused':
      return { accent: 'var(--warning)', icon: <Pause {...ICON_PROPS} /> };
    case 'completed':
      return { accent: 'var(--success)', icon: <CheckOne {...ICON_PROPS} /> };
    case 'failed':
      return { accent: 'var(--danger)', icon: <CloseOne {...ICON_PROPS} /> };
    case 'cancelled':
      return { accent: 'var(--bg-6)', icon: <CloseOne {...ICON_PROPS} /> };
    case 'error':
      return { accent: 'var(--danger)', icon: <Caution {...ICON_PROPS} /> };
    case 'idle':
    default:
      return { accent: 'var(--bg-6)', icon: <Peoples {...ICON_PROPS} /> };
  }
}

/**
 * Read the current workspace-rail collapse state by listening for the
 * `WORKSPACE_STATE_EVENT` the rail broadcasts whenever it (re)renders or toggles.
 * Starts `true` (rail defaults to collapsed) until the first broadcast arrives.
 */
function useRailCollapsed(): React.MutableRefObject<boolean> {
  const collapsedRef = useRef(true);
  useEffect(() => {
    if (typeof window === 'undefined') return undefined;
    const handler = (event: Event) => {
      collapsedRef.current = (event as CustomEvent<WorkspaceStateDetail>).detail?.collapsed ?? true;
    };
    window.addEventListener(WORKSPACE_STATE_EVENT, handler);
    return () => window.removeEventListener(WORKSPACE_STATE_EVENT, handler);
  }, []);
  return collapsedRef;
}

/**
 * OrchestrationStatusStrip — a compact, always-visible strip rendered at the top
 * of a lead conversation. It surfaces the orchestration state in EVERY case
 * (规划中 / 待批准 / 协作中 / 完成 / 失败 / 未拆分 / 未配置模型) so the user can see that
 * auto-orchestration is happening, and gives a one-click path into the DAG rail.
 *
 * Behavior:
 *  - Renders nothing unless `conversation.extra.orchestrator_role === 'lead'`.
 *  - Clicking the strip opens the workspace rail and selects the 「编排」DAG tab.
 *  - `awaiting` shows a primary 「查看并批准」CTA (same target — opens the rail
 *    where the approve button lives).
 *  - `error` shows a subtle 「去配置模型」link → /settings/model.
 *  - When a run first appears (and on entering awaiting/running), the rail is
 *    auto-revealed ONCE per run id (a later manual collapse is respected).
 */
const OrchestrationStatusStrip: React.FC<{ conversation: TChatConversation | undefined }> = ({ conversation }) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const status = useOrchestrationStatus(conversation);
  const railCollapsedRef = useRailCollapsed();

  // Track which run ids we have already auto-revealed for, so we open the rail
  // exactly once per run and never fight a user who later collapses it.
  const autoRevealedRunsRef = useRef<Set<string>>(new Set());

  const revealRail = useCallback(() => {
    if (railCollapsedRef.current) {
      dispatchWorkspaceToggleEvent();
    }
    // Whether or not it was collapsed, make sure the DAG tab is the active one.
    dispatchWorkspaceSelectTabEvent(DAG_TAB_KEY);
  }, [railCollapsedRef]);

  // Auto-reveal once per run when it first exists / enters awaiting|running.
  const phase = status?.phase;
  const runId = status?.runId;
  useEffect(() => {
    if (!runId) return undefined;
    if (phase !== 'awaiting' && phase !== 'running' && phase !== 'planning') return undefined;
    if (autoRevealedRunsRef.current.has(runId)) return undefined;
    autoRevealedRunsRef.current.add(runId);
    // Defer one macrotask so the rail's initial WORKSPACE_STATE_EVENT broadcast
    // settles `railCollapsedRef` first — otherwise a first-render race could
    // toggle (and thus collapse) a rail the user had left expanded.
    const id = setTimeout(() => revealRail(), 0);
    return () => clearTimeout(id);
  }, [runId, phase, revealRail]);

  const visual = useMemo(() => (status ? phaseVisual(status.phase) : null), [status]);

  if (!status || !visual) return null;

  const { accent } = visual;
  const isAwaiting = status.phase === 'awaiting';
  const isError = status.phase === 'error';
  const showProgress = status.phase === 'running' && status.total > 0;

  // Phase label / detail copy. running carries an X/Y suffix; awaiting/completed
  // carry a task count. count is always supplied so `{{count}}` never leaks even
  // when the plan's tasks have not loaded yet (total === 0).
  const label =
    status.phase === 'running'
      ? t('orchestrator.status.running', { done: status.done, total: status.total })
      : status.phase === 'awaiting'
        ? t('orchestrator.status.awaiting', { count: status.total })
        : status.phase === 'completed'
          ? t('orchestrator.status.completed', { count: status.total })
          : t(`orchestrator.status.${status.phase}`);

  const onActivate = () => revealRail();
  const onKeyActivate = (e: React.KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      revealRail();
    }
  };

  return (
    <div
      role='button'
      tabIndex={0}
      aria-label={label}
      onClick={onActivate}
      onKeyDown={onKeyActivate}
      className='group mx-12px mt-8px mb-4px flex shrink-0 cursor-pointer select-none items-center gap-8px rd-10px px-12px py-7px transition-colors'
      style={{
        // Soft tinted surface — NOT a loud color block. The accent only bleeds
        // through at low opacity via color-mix, with a hairline border to match.
        background: `color-mix(in srgb, ${accent} 8%, var(--bg-2))`,
        border: `1px solid color-mix(in srgb, ${accent} 22%, transparent)`,
      }}
    >
      {/* Status dot + icon */}
      <span className='relative flex size-18px shrink-0 items-center justify-center'>
        <span
          className='absolute inset-0 rd-full opacity-15'
          style={{ background: accent }}
          aria-hidden
        />
        <span
          className={visual.spin ? 'flex animate-spin' : 'flex'}
          style={{ color: accent, lineHeight: 0 }}
        >
          {visual.icon}
        </span>
      </span>

      {/* Label */}
      <span className='min-w-0 flex-1 truncate text-13px font-500 text-t-primary'>{label}</span>

      {/* Inline mini progress for the running phase */}
      {showProgress && (
        <span className='hidden shrink-0 items-center gap-6px sm:flex'>
          <span className='h-5px w-72px overflow-hidden rd-full' style={{ background: 'var(--bg-3)' }}>
            <span
              className='block h-full rd-full transition-all duration-300'
              style={{
                width: `${status.total > 0 ? Math.round((status.done / status.total) * 100) : 0}%`,
                background: accent,
              }}
            />
          </span>
          <span className='text-11px font-500 tabular-nums text-t-tertiary'>
            {status.done}/{status.total}
          </span>
        </span>
      )}

      {/* Awaiting → primary 「查看并批准」CTA (opens the rail, where approve lives) */}
      {isAwaiting && (
        <span
          className='flex h-24px shrink-0 items-center gap-4px rd-6px px-9px text-12px font-500 text-white transition-opacity group-hover:opacity-90'
          style={{ background: accent }}
        >
          <span>{t('orchestrator.status.reviewApprove')}</span>
          <Right theme='outline' size='13' strokeWidth={3} />
        </span>
      )}

      {/* Error → subtle 「去配置模型」link */}
      {isError && (
        <span
          role='button'
          tabIndex={0}
          aria-label={t('orchestrator.status.configureModel')}
          onClick={(e) => {
            e.stopPropagation();
            void navigate('/settings/model');
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault();
              e.stopPropagation();
              void navigate('/settings/model');
            }
          }}
          className='shrink-0 text-12px font-500 underline-offset-2 hover:underline'
          style={{ color: accent }}
        >
          {t('orchestrator.status.configureModel')}
        </span>
      )}

      {/* Trailing affordance for non-CTA phases hints "click to open DAG". */}
      {!isAwaiting && !isError && (
        <Right
          theme='outline'
          size='14'
          strokeWidth={3}
          className='shrink-0 text-t-tertiary transition-transform group-hover:translate-x-2px'
        />
      )}
    </div>
  );
};

export default OrchestrationStatusStrip;
