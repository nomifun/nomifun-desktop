import { IconCheckCircle, IconExclamationCircle } from '@arco-design/web-react/icon';
import { Loading } from '@icon-park/react';
import { useId, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { TaskPlanSnapshot } from '@/common/protocolBindings/TaskPlanSnapshot';
import { deriveTaskPlan } from './taskPlanModel';

const statusKeys = {
  pending: 'messages.planPending', in_progress: 'messages.planInProgress',
  completed: 'messages.planCompleted', blocked: 'messages.planBlocked',
} as const;

/** Current task progress, supplied by the canonical runtime snapshot. */
export default function TaskPlanBar({ snapshot }: { snapshot?: TaskPlanSnapshot | null }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const listId = useId();
  const plan = deriveTaskPlan(snapshot);
  if (!plan) return null;
  const stateLabel = plan.paused ? t('messages.planPaused', { defaultValue: 'Paused' })
    : plan.stopped ? t('messages.planStopped', { defaultValue: 'Stopped' })
    : plan.failed ? t('messages.planFailed', { defaultValue: 'Failed' })
    : plan.needsReplan ? t('messages.planNeedsReplan', { defaultValue: 'Plan needs updating' })
    : plan.blocked > 0 ? t('messages.planBlocked', { defaultValue: 'Blocked' }) : null;

  return (
    <div
      data-testid='task-plan-bar'
      className='relative w-fit max-w-[calc(100vw-32px)]'
      onMouseEnter={() => setExpanded(true)}
      onMouseLeave={() => setExpanded(false)}
      onFocus={() => setExpanded(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setExpanded(false);
      }}
      onKeyDown={(event) => { if (event.key === 'Escape') setExpanded(false); }}
    >
      <button
        type='button'
        aria-expanded={expanded}
        aria-controls={listId}
        data-testid='task-plan-summary'
        className='flex h-28px items-center gap-6px rd-999px px-10px cursor-pointer select-none'
        style={{
          background: 'var(--color-bg-1)',
          border: '1px solid color-mix(in srgb, rgb(var(--primary-6)) 14%, var(--color-border-2))',
          color: 'var(--text-secondary)',
        }}
        onClick={() => setExpanded(true)}
      >
        {plan.active && plan.steps.some((step) => step.status === 'in_progress') && !plan.needsReplan && (
          <Loading aria-hidden='true' data-testid='task-plan-progress-indicator' theme='outline' size='14'
            className='shrink-0 animate-spin' />
        )}
        <span className='min-w-0 truncate text-12px font-600 leading-none'>
          {t('messages.planTodoList', { defaultValue: 'Task list' })}
        </span>
        <span className='ml-18px whitespace-nowrap text-12px leading-none tabular-nums'>
          {t('messages.planProgress', { done: plan.done, total: plan.total, defaultValue: '{{done}}/{{total}}' })}
        </span>
        {stateLabel && <span className='whitespace-nowrap text-12px'>{stateLabel}</span>}
      </button>
      {expanded && (
        <div className='absolute left-1/2 bottom-full pb-8px w-[min(360px,calc(100vw-32px))] -translate-x-1/2 z-10'>
          <div className='flex max-h-[260px] flex-col gap-8px overflow-y-auto rd-12px px-12px py-10px'
            style={{ background: 'var(--color-bg-2)', border: '1px solid var(--color-border-2)',
              boxShadow: '0 8px 22px rgba(15, 23, 42, 0.08)' }}>
            {plan.explanation && <p className='m-0 text-12px leading-18px text-t-secondary'>{plan.explanation}</p>}
            <ul id={listId} data-testid='task-plan-list' className='m-0 p-0 flex flex-col gap-6px list-none'>
              {plan.steps.map((item, index) => (
                <li key={index} data-status={item.status} aria-label={`${t(statusKeys[item.status])}: ${item.step}`}
                  className='flex min-h-22px items-start gap-8px text-12px leading-18px text-t-secondary'>
                  {item.status === 'completed' ? (
                    <IconCheckCircle aria-hidden='true' fontSize={18} className='shrink-0 text-success' />
                  ) : item.status === 'blocked' ? (
                    <IconExclamationCircle aria-hidden='true' fontSize={18} className='shrink-0 text-warning' />
                  ) : (
                    <span aria-hidden='true' className='size-18px flex shrink-0 items-center justify-center'>
                      <span className='size-11px rd-full b-2px b-solid'
                        style={{ borderColor: item.status === 'in_progress' ? 'rgb(var(--primary-6))' : 'var(--color-border-3)' }} />
                    </span>
                  )}
                  <span className={`min-w-0 flex-1 break-words ${item.status === 'in_progress' ? 'text-t-primary' : ''}`}>
                    {item.step}
                    {item.status === 'blocked' && <span className='ml-6px'>
                      {t('messages.planBlocked', { defaultValue: 'Blocked' })}
                    </span>}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        </div>
      )}
    </div>
  );
}
