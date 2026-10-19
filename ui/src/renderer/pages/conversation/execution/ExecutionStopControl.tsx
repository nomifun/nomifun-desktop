import React, { useRef, useState } from 'react';
import { Loading, Pause } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { ExecutionId } from '@/common/types/ids';
import type { TExecutionAutomationControl } from '@/common/types/agentExecution/agentExecutionTypes';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { isTerminalExecutionStatus } from './executionStatusMeta';
import { refreshOnVersionConflict } from './refreshOnVersionConflict';
import AutomationRunStopControl from './AutomationRunStopControl';

/** Cancels the exact collaboration, including queued or suspended attempts. */
const ExecutionStopControl: React.FC<{
  executionId: ExecutionId;
  executionVersion: number;
  status: string;
  refetch: () => Promise<void>;
  disabled?: boolean;
  onBusyChange?: (busy: boolean) => void;
  automationControl?: TExecutionAutomationControl | null;
}> = ({ executionId, executionVersion, status, refetch, disabled = false, onBusyChange, automationControl }) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage();
  const [stopping, setStopping] = useState(false);
  const [confirmedCancelledId, setConfirmedCancelledId] = useState<ExecutionId | null>(null);
  const inFlight = useRef(false);
  if (automationControl) return <AutomationRunStopControl key={executionId} control={automationControl}
    status={status} refetch={refetch} disabled={disabled} onBusyChange={onBusyChange} />;
  if (!status || isTerminalExecutionStatus(status) || confirmedCancelledId === executionId) return <>{messageContext}</>;

  const stop = async () => {
    if (disabled || inFlight.current) return;
    inFlight.current = true;
    setStopping(true);
    onBusyChange?.(true);
    try {
      const result = await ipcBridge.agentExecution.cancel.invoke({
        execution_id: executionId,
        updates: { expected_version: executionVersion },
      });
      if (result.execution.execution_id !== executionId || result.execution.status !== 'cancelled') {
        throw new Error('Cancellation did not return the stopped collaboration');
      }
      setConfirmedCancelledId(executionId);
      message.success(t('agentExecution.controls.cancelOk'));
      try {
        await refetch();
      } catch {
        // The stop already committed. A stale display must not invite repeating it.
        message.warning(t('agentExecution.detail.loadError'));
      }
    } catch (error) {
      await refreshOnVersionConflict(error, refetch);
      message.error(t('agentExecution.controls.cancelError', { error: String(error) }));
    } finally {
      inFlight.current = false;
      setStopping(false);
      onBusyChange?.(false);
    }
  };
  const busy = disabled || stopping;
  const label = t(stopping ? 'agentExecution.controls.stopping' : 'agentExecution.controls.cancel');
  return <>
    {messageContext}
    <div role='button' tabIndex={busy ? -1 : 0} aria-label={label} aria-disabled={busy} aria-busy={stopping}
      onClick={busy ? undefined : () => void stop()}
      onKeyDown={event => {
        if (busy || (event.key !== 'Enter' && event.key !== ' ')) return;
        event.preventDefault();
        void stop();
      }}
      className='flex h-30px shrink-0 cursor-pointer select-none items-center gap-5px rd-8px border border-solid border-[var(--border-base)] px-10px text-12px font-500 text-t-secondary transition-all duration-150 hover:border-danger hover:text-danger'
      style={{ opacity: busy ? 0.6 : undefined, pointerEvents: busy ? 'none' : undefined }}>
      {stopping ? <Loading theme='outline' size='14' className='animate-spin line-height-0' /> : <Pause theme='outline' size='14' />}
      <span>{label}</span>
    </div>
  </>;
};

export default ExecutionStopControl;
