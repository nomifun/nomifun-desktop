import React, { useRef, useState } from 'react';
import { Loading, Pause } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { agentActivities } from '@/common/adapter/ipcBridge';
import type { AgentActivityTarget } from '@/common/types/agentExecution/agentActivity';
import type { TExecutionAutomationControl } from '@/common/types/agentExecution/agentExecutionTypes';
import { uuidv7 } from '@/common/utils/uuidv7';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { isTerminalExecutionStatus } from './executionStatusMeta';
import { refreshOnVersionConflict } from './refreshOnVersionConflict';

/** Stops the captured run through its Requirement owner, including from a child view. */
const AutomationRunStopControl: React.FC<{
  control: TExecutionAutomationControl;
  status: string;
  refetch: () => Promise<void>;
  disabled?: boolean;
  onBusyChange?: (busy: boolean) => void;
}> = ({ control, status, refetch, disabled = false, onBusyChange }) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage();
  const [stopping, setStopping] = useState(false);
  const [acceptedIdentity, setAcceptedIdentity] = useState<string | null>(null);
  const keys = useRef(new Map<string, string>());
  const inFlight = useRef(false);
  const target: AgentActivityTarget = {
    kind: 'auto_work_run', target_kind: 'conversation', target_id: control.lead_conversation_id,
    requirement_id: control.requirement_id, claim_generation: control.claim_generation,
  };
  const identity = JSON.stringify(target);
  if (!status || isTerminalExecutionStatus(status)) return <>{messageContext}</>;
  if (acceptedIdentity === identity) return <>{messageContext}<span role='status' className='text-12px text-t-secondary'>
    {t('agentExecution.taskStop.accepted')}
  </span></>;

  const stop = async () => {
    if (disabled || inFlight.current) return;
    inFlight.current = true;
    setStopping(true);
    onBusyChange?.(true);
    const key = keys.current.get(identity) ?? uuidv7();
    keys.current.set(identity, key);
    try {
      const receipt = await agentActivities.stop.invoke({ target, idempotency_key: key });
      if (!receipt.accepted) throw new Error('Automatic run stop was not accepted by its owner');
      setAcceptedIdentity(identity);
      message.success(t('agentExecution.taskStop.accepted'));
      try { await refetch(); } catch { message.warning(t('agentExecution.detail.loadError')); }
    } catch (error) {
      await refreshOnVersionConflict(error, refetch);
      message.error(t('agentExecution.controls.cancelError', { error: String(error) }));
    } finally {
      inFlight.current = false;
      setStopping(false);
      onBusyChange?.(false);
    }
  };
  return <>
    {messageContext}
    <button type='button' disabled={disabled || stopping} aria-busy={stopping} onClick={() => void stop()}
      className='flex h-30px shrink-0 cursor-pointer select-none items-center gap-5px rd-8px border border-solid border-[var(--border-base)] px-10px text-12px font-500 text-t-secondary transition-all duration-150 hover:border-danger hover:text-danger'>
      {stopping ? <Loading theme='outline' size='14' className='animate-spin line-height-0' /> : <Pause theme='outline' size='14' />}
      {t(stopping ? 'agentExecution.controls.stopping' : 'agentExecution.controls.stopAutomaticRun')}
    </button>
  </>;
};

export default AutomationRunStopControl;
