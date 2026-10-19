import { Dropdown, Menu } from '@arco-design/web-react';
import { Down, Loading, Pause } from '@icon-park/react';
import React, { useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { ExecutionId, ExecutionStepId } from '@/common/types/ids';
import type { TExecutionCancellationScope } from '@/common/types/agentExecution/agentExecutionTypes';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { refreshOnVersionConflict } from './refreshOnVersionConflict';
import { isTerminalExecutionStatus } from './executionStatusMeta';

const controlClass = 'flex h-30px shrink-0 cursor-pointer select-none items-center gap-5px rd-8px border border-solid border-[var(--border-base)] px-10px text-12px font-500 text-t-secondary transition-all duration-150 hover:border-danger hover:text-danger';

/** Task acceptance is separate from native stop and resource release. */
const ExecutionStepStopControl: React.FC<{
  executionId: ExecutionId;
  executionVersion: number;
  executionStatus: string;
  stepId: ExecutionStepId;
  stepVersion: number;
  stepStatus: string;
  delegatedOnly?: boolean;
  refetch: () => Promise<void>;
}> = ({ executionId, executionVersion, executionStatus, stepId, stepVersion, stepStatus, delegatedOnly = false, refetch }) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage();
  const [stopping, setStopping] = useState(false);
  const [acceptedStepId, setAcceptedStepId] = useState<ExecutionStepId | null>(null);
  const [acceptedDelegatedStepId, setAcceptedDelegatedStepId] = useState<ExecutionStepId | null>(null);
  const inFlight = useRef(false);
  const taskAccepted = acceptedStepId === stepId || stepStatus === 'cancelled';
  const accepted = taskAccepted || acceptedDelegatedStepId === stepId;
  const canStopTask = !delegatedOnly && !taskAccepted && !['completed', 'failed', 'skipped'].includes(stepStatus);
  const canStopDelegated = Boolean(executionStatus) && !isTerminalExecutionStatus(executionStatus)
    && acceptedDelegatedStepId !== stepId;
  if (!canStopDelegated && !accepted) return <>{messageContext}</>;

  const stop = async (scope: TExecutionCancellationScope) => {
    if (inFlight.current || (delegatedOnly && scope !== 'delegated_work')) return;
    inFlight.current = true;
    setStopping(true);
    try {
      const result = await ipcBridge.agentExecution.cancelStep.invoke({
        execution_id: executionId,
        step_id: stepId,
        updates: {
          expected_execution_version: executionVersion,
          expected_step_version: stepVersion,
          scope,
        },
      });
      const stoppedStep = result.steps.find(step => step.step_id === stepId);
      const targetAccepted = stoppedStep && (scope === 'delegated_work'
        || stoppedStep.status === 'cancelled' || (scope === 'task_and_delegated'
          && ['completed', 'failed', 'skipped'].includes(stoppedStep.status)));
      if (result.execution.execution_id !== executionId || !targetAccepted) {
        throw new Error('Task stop did not return its accepted target');
      }
      if (scope !== 'delegated_work') setAcceptedStepId(stepId);
      if (scope !== 'task') setAcceptedDelegatedStepId(stepId);
      message.success(t('agentExecution.taskStop.accepted'));
      try {
        await refetch();
      } catch {
        message.warning(t('agentExecution.detail.loadError'));
      }
    } catch (error) {
      await refreshOnVersionConflict(error, refetch);
      message.error(t('agentExecution.taskStop.error', { error: String(error) }));
    } finally {
      inFlight.current = false;
      setStopping(false);
    }
  };
  const menu = <Menu onClickMenuItem={key => void stop(key as TExecutionCancellationScope)}>
    <Menu.Item key='delegated_work' disabled={stopping}>{t('agentExecution.taskStop.delegatedOnly')}</Menu.Item>
    <Menu.Item key='task_and_delegated' disabled={stopping}>{t('agentExecution.taskStop.includeDelegated')}</Menu.Item>
  </Menu>;
  return <>
    {messageContext}
    <div className='flex max-w-full flex-wrap items-center gap-4px'>
      {accepted && <span role='status' className='text-12px text-t-secondary'>
        {t('agentExecution.taskStop.accepted')}
      </span>}
      {canStopTask && <button type='button' disabled={stopping} aria-busy={stopping}
        className={controlClass} onClick={() => void stop('task')}>
        {stopping ? <Loading theme='outline' size='14' className='animate-spin line-height-0' /> : <Pause theme='outline' size='14' />}
        {t(stopping ? 'agentExecution.controls.stopping' : 'agentExecution.taskStop.task')}
      </button>}
      {canStopDelegated && delegatedOnly && <button type='button' disabled={stopping} aria-busy={stopping}
        className={controlClass} onClick={() => void stop('delegated_work')}>
        {t('agentExecution.taskStop.delegatedOnly')}
      </button>}
      {canStopDelegated && !delegatedOnly && <Dropdown trigger='click' droplist={menu} disabled={stopping}>
        <button type='button' disabled={stopping} aria-label={t('agentExecution.taskStop.scope')} className={controlClass}>
          <Down theme='outline' size='12' />
        </button>
      </Dropdown>}
    </div>
  </>;
};

export default ExecutionStepStopControl;
