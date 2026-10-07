/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Spin } from '@arco-design/web-react';
import { Branch, CheckOne, Right } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { latestAttemptForStep } from '@/common/types/agentExecution/agentExecutionTypes';
import type { OpenStepPayload } from '../../execution/DagCanvas';
import type { ConversationDelegation } from '../conversationDelegationModel';

/** Read-only progress; opening a task reuses the existing attempt transcript projection. */
const DelegationProgress: React.FC<{
  delegation: ConversationDelegation;
  projectStep: (payload: OpenStepPayload) => void;
  refetch: () => Promise<void>;
}> = ({ delegation, projectStep, refetch }) => {
  const { t } = useTranslation();
  const detail = delegation.detail;
  const steps = detail?.steps.filter(step => step.superseded_in_revision === null) ?? [];
  const completed = steps.filter(step => step.status === 'completed').length;
  return <section className='conversation-delegation' data-testid='conversation-delegation'>
    <div className='conversation-delegation__heading'>
      <Branch theme='outline' size='15' />
      <span>{t('messages.delegation.title')}</span>
      {steps.length > 0 && <span className='conversation-delegation__count'>
        {t('agentExecution.progress.summary', { done: completed, total: steps.length })}
      </span>}
    </div>
    {!steps.length && <div className='conversation-delegation__empty' role='status'>
      {delegation.unfinished && <Spin size={12} />}
      <span>{detail ? t(`agentExecution.status.execution.${detail.execution.status}`) : t('messages.delegation.syncing')}</span>
      {detail && <span className='conversation-delegation__goal'>{detail.execution.goal}</span>}
    </div>}
    {steps.map(step => {
      const attempt = latestAttemptForStep(detail?.attempts ?? [], step.step_id);
      const participant = detail?.participants.find(value => value.participant_id === (attempt?.participant_id ?? step.assigned_participant_id));
      const participantLabel = [participant?.role?.trim() || step.role?.trim(), participant?.model?.trim()].filter(Boolean).join(' · ');
      const canOpen = Boolean(attempt?.conversation_id);
      const Header = canOpen ? 'button' : 'div';
      const status = attempt?.status === 'queued' && step.status === 'running' ? 'pending' : step.status;
      return <div key={step.step_id} className={`conversation-delegation__task conversation-delegation__task--${status}`}>
        <Header type={canOpen ? 'button' : undefined} className='conversation-delegation__row'
          onClick={canOpen ? () => projectStep({ step, attempt, participant, participants: detail?.participants ?? [],
            executionId: delegation.executionId, refetch }) : undefined}
          aria-label={canOpen ? t('messages.delegation.openTask', { title: step.title }) : undefined}>
          <span className='conversation-delegation__icon' aria-hidden='true'>
            {status === 'running' ? <Spin size={12} /> : status === 'completed' ? <CheckOne theme='outline' size='14' /> : <Branch theme='outline' size='14' />}
          </span>
          <span className='conversation-delegation__content'>
            <span className='conversation-delegation__name'>{step.title}</span>
            {participantLabel && <span className='conversation-delegation__participant'>{participantLabel}</span>}
          </span>
          <span className='conversation-delegation__status'>{t(`agentExecution.status.step.${status}`)}</span>
          {canOpen && <Right theme='outline' size='12' />}
        </Header>
        {step.status === 'waiting_input' && attempt?.question && <p className='conversation-delegation__note'>{attempt.question}</p>}
        {step.status === 'failed' && attempt?.error && <p className='conversation-delegation__note'>{attempt.error}</p>}
      </div>;
    })}
  </section>;
};

export default DelegationProgress;
