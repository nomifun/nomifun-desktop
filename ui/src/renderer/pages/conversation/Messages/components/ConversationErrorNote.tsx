/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentStreamErrorInfo } from '@/common/chat/chatLib';
import { normalizeModelFailureDiagnostic } from '@/common/chat/providerDiagnostic';
import type { ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import { Attention, Down } from '@icon-park/react';
import React, { useId, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import FeedbackButton from '@renderer/components/base/FeedbackButton';
import CopyIconButton from '@renderer/components/base/CopyIconButton';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import GatewayBillingAction, { gatewayActionError } from './GatewayBillingAction';
import '../messages.css';

interface ConversationErrorNoteProps {
  error?: AgentStreamErrorInfo;
  rawDetail?: string;
  timestamp?: number;
  turnId?: string;
  sessionId?: string;
  messageId?: string;
  creationTaskId?: string;
  recoveryAction?: ReactNode;
  currentModel?: ConversationContextValue['currentModel'];
  feedback?: boolean;
}

/** Shared presentation for canonical failures, pauses and rejected requests. */
const ConversationErrorNote: React.FC<ConversationErrorNoteProps> = ({
  error, rawDetail = error?.detail || error?.message, timestamp, turnId, sessionId,
  messageId, creationTaskId, recoveryAction, currentModel, feedback = true,
}) => {
  const { t } = useTranslation();
  const [detailsExpanded, setDetailsExpanded] = useState(false);
  const detailsId = useId();
  const detailsLabel = t(detailsExpanded ? 'conversation.agentError.collapseDetails' : 'conversation.agentError.expandDetails');
  const code = error?.code;
  const diagnostic = normalizeModelFailureDiagnostic(error?.providerDiagnostic);
  const reason = code === 'NOMIFUN_TASK_INCOMPLETE' ? error?.taskIncompleteReason : undefined;
  const copyKey = diagnostic
    ? `conversation.agentError.providerReasons.${diagnostic.reason}`
    : reason ? `conversation.agentError.incompleteReasons.${reason}`
      : code ? `conversation.agentError.codes.${code}` : undefined;
  const title = copyKey
    ? t(`${copyKey}.title`, {
        defaultValue: t('conversation.agentError.fallbackTitle'),
      })
    : t('conversation.agentError.fallbackTitle');
  const hasPathExplanation = !diagnostic && error?.workspacePath
    && (code === 'WORKSPACE_PATH_EDGE_WHITESPACE_RUNTIME_UNSUPPORTED' || code === 'WORKSPACE_DIRECTORY_RUNTIME_UNAVAILABLE');
  const body = copyKey
    ? t(
        `${copyKey}.${hasPathExplanation ? 'bodyWithPath' : 'body'}`,
        {
          workspacePath: error?.workspacePath,
          defaultValue: t('conversation.agentError.fallbackBody'),
        }
      )
    : t('conversation.agentError.fallbackBody');
  const agentLabel = error?.agentTemplateKey
    ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[error.agentTemplateKey]}.name`)
    : error?.agentLabel;
  const date = timestamp !== undefined ? new Date(timestamp) : undefined;
  const metadata = [
    [t('conversation.agentError.details.agent'), agentLabel],
    [t('conversation.agentError.details.model'), diagnostic ? diagnostic.modelName : error?.modelName],
    ...(diagnostic && error?.modelName && diagnostic.modelName !== error.modelName
      ? [[t('conversation.agentError.details.sessionModel'), error.modelName]] : []),
    ...([
      ['httpStatus', diagnostic?.httpStatus],
      ['providerId', diagnostic?.providerId],
      ['endpoint', diagnostic?.endpoint],
      ['protocol', diagnostic?.protocol],
      ['authScheme', diagnostic?.authScheme],
      ['providerCode', diagnostic?.providerCode],
      ['providerType', diagnostic?.providerType],
      ['providerParam', diagnostic?.providerParam],
      ['requestId', diagnostic?.requestId],
      ['retryAfterMs', diagnostic?.retryAfterMs],
      ['contentType', diagnostic?.contentType],
    ] as const).filter(([, value]) => value !== undefined)
      .map(([label, value]) => [t(`conversation.agentError.details.${label}`), String(value)]),
    [t('conversation.agentError.details.timestamp'), date && Number.isFinite(date.getTime()) ? date.toISOString() : undefined],
    [t('conversation.agentError.details.turn'), turnId],
    [t('conversation.agentError.details.session'), sessionId],
    ...(messageId ? [[t('conversation.agentError.details.message'), messageId]] : []),
    ...(creationTaskId ? [[t('conversation.agentError.details.creationTask'), creationTaskId]] : []),
    [t('conversation.agentError.details.workspace'), error?.workspacePath],
  ];
  const diagnosticDetail = diagnostic?.transportDetail || rawDetail;
  const report = [title, body, code ? `code: ${code}` : '', diagnostic ? `reason: ${diagnostic.reason}` : '',
    ...metadata.map(([label, value]) => `${label}: ${value || t('conversation.agentError.details.unavailable')}`),
    diagnosticDetail ? `${t('conversation.agentError.details.rawDetail')}:\n${diagnosticDetail}` : '',
  ].filter(Boolean).join('\n');

  return (
    <div className='message-error-note'>
      <div className='message-error-note__summary'>
        <span className='message-error-note__icon' aria-hidden='true'>
          <Attention theme='outline' size='16' fill='currentColor' />
        </span>
        <span className='message-error-note__title' role='alert'>
          {title}
        </span>
      </div>
      <div className='message-error-note__guidance'>
        <div className='message-error-note__body'>{body}</div>
        {gatewayActionError(code) && <GatewayBillingAction code={code} reason={diagnostic?.reason} providerId={diagnostic?.providerId} model={currentModel} />}
        <div className='message-error-note__controls'>
          {recoveryAction}
          <button
            type='button'
            className='message-error-note__toggle'
            aria-expanded={detailsExpanded}
            aria-controls={detailsId}
            onClick={() => setDetailsExpanded((expanded) => !expanded)}
          >
            {detailsLabel}
            <Down theme='outline' size='14' fill='currentColor' aria-hidden='true' />
          </button>
          {detailsExpanded && <CopyIconButton text={report} tooltip={t('conversation.agentError.copyDetails')} className='message-error-note__copy' />}
        </div>
      </div>
      <div
        id={detailsId}
        className='message-error-note__details'
        hidden={!detailsExpanded}
        role='region'
        aria-label={t('conversation.agentError.expandDetails')}
      >
        <dl className='message-error-note__context'>
          {metadata.map(([label, value]) => (
            <div className='message-error-note__context-row' key={label}>
              <dt>{label}</dt>
              <dd>{value || t('conversation.agentError.details.unavailable')}</dd>
            </div>
          ))}
        </dl>
        {diagnosticDetail && (
          <div className='message-error-note__diagnostic'>
            <div className='message-error-note__detail-label'>{t('conversation.agentError.details.rawDetail')}</div>
            <pre className='message-error-note__detail-body'>{diagnosticDetail}</pre>
          </div>
        )}
        {feedback && <div className='message-error-note__actions'>
          <FeedbackButton className='message-error-note__feedback' />
        </div>}
      </div>
    </div>
  );
};

export default ConversationErrorNote;
