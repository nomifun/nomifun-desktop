/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentStreamErrorInfo } from '@/common/chat/chatLib';
import { normalizeModelFailureDiagnostic } from '@/common/chat/providerDiagnostic';
import { normalizeSystemExecutionFailure } from '@/common/chat/systemExecutionError';
import type { ConversationContextValue } from '@/renderer/hooks/context/ConversationContext';
import { Attention, Down } from '@icon-park/react';
import React, { useId, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import FeedbackButton from '@renderer/components/base/FeedbackButton';
import CopyIconButton from '@renderer/components/base/CopyIconButton';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import GatewayBillingAction, { gatewayActionError } from './GatewayBillingAction';
import ErrorDiagnosticDetails, { modelFailureDiagnosticRows, type DiagnosticMetadataRow } from './ErrorDiagnosticDetails';

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
  /** Task blocker stays visible without competing with an adjacent recovery card. */
  compact?: boolean;
  tone?: 'error' | 'notice';
}

/** Shared presentation for canonical failures, pauses and rejected requests. */
const ConversationErrorNote: React.FC<ConversationErrorNoteProps> = ({
  error, rawDetail = error?.detail || error?.message, timestamp, turnId, sessionId,
  messageId, creationTaskId, recoveryAction, currentModel, feedback = true, compact = false, tone = 'error',
}) => {
  const { t } = useTranslation();
  const [detailsExpanded, setDetailsExpanded] = useState(false);
  const detailsId = useId();
  const detailsLabel = t(detailsExpanded ? 'conversation.agentError.collapseDetails' : 'conversation.agentError.expandDetails');
  const code = error?.code;
  const diagnostic = normalizeModelFailureDiagnostic(error?.providerDiagnostic);
  const systemError = normalizeSystemExecutionFailure(error?.systemExecutionError);
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
  const metadata: DiagnosticMetadataRow[] = [
    [t('conversation.agentError.details.agent'), agentLabel],
    [t('conversation.agentError.details.model'), diagnostic ? diagnostic.modelName : error?.modelName],
    ...(diagnostic && error?.modelName && diagnostic.modelName !== error.modelName
      ? [[t('conversation.agentError.details.sessionModel'), error.modelName] as const] : []),
    ...modelFailureDiagnosticRows(diagnostic, label => t(`conversation.agentError.details.${label}`)),
    ...(systemError ? [
      [t('conversation.agentError.details.nativeCode'), systemError.code],
      [t('conversation.agentError.details.nativeKind'), systemError.kind ?? undefined],
      [t('conversation.agentError.details.nativeOsError'), systemError.raw_os_error?.toString()],
      [t('conversation.agentError.details.userCodeNotStarted'), systemError.user_code_not_started?.toString()],
      [t('conversation.agentError.details.cleanupProven'), systemError.cleanup_proven?.toString()],
    ] as DiagnosticMetadataRow[] : []),
    [t('conversation.agentError.details.timestamp'), date && Number.isFinite(date.getTime()) ? date.toISOString() : undefined],
    [t('conversation.agentError.details.turn'), turnId],
    [t('conversation.agentError.details.session'), sessionId],
    ...(messageId ? [[t('conversation.agentError.details.message'), messageId] as const] : []),
    ...(creationTaskId ? [[t('conversation.agentError.details.creationTask'), creationTaskId] as const] : []),
    [t('conversation.agentError.details.workspace'), error?.workspacePath],
  ];
  const diagnosticDetail = systemError?.message || diagnostic?.transportDetail || rawDetail;
  const report = [title, body, code ? `code: ${code}` : '', diagnostic ? `reason: ${diagnostic.reason}` : '',
    ...metadata.map(([label, value]) => `${label}: ${value || t('conversation.agentError.details.unavailable')}`),
    diagnosticDetail ? `${t('conversation.agentError.details.rawDetail')}:\n${diagnosticDetail}` : '',
  ].filter(Boolean).join('\n');

  const controls = <div className='message-error-note__controls'>
    {recoveryAction}
    <button type='button' className='message-error-note__toggle' aria-expanded={detailsExpanded} aria-controls={detailsId}
      onClick={() => setDetailsExpanded((expanded) => !expanded)}>
      {detailsLabel}<Down theme='outline' size='14' fill='currentColor' aria-hidden='true' />
    </button>
    {detailsExpanded && <CopyIconButton text={report} tooltip={t('conversation.agentError.copyDetails')} className='message-error-note__copy' />}
  </div>;
  return (
    <div className={`message-error-note${compact ? ' message-error-note--compact' : ''}${tone === 'notice' ? ' message-error-note--notice' : ''}`}>
      <div className='message-error-note__summary'>
        <span className='message-error-note__icon' aria-hidden='true'>
          <Attention theme='outline' size='16' fill='currentColor' />
        </span>
        <span className='message-error-note__title' role={tone === 'notice' ? 'note' : 'alert'}>
          {title}
        </span>
        {compact && controls}
      </div>
      {(!compact || detailsExpanded) && <div className='message-error-note__guidance'>
        <div className='message-error-note__body'>{body}</div>
        {gatewayActionError(code) && <GatewayBillingAction code={code} reason={diagnostic?.reason} providerId={diagnostic?.providerId} model={currentModel} />}
        {!compact && controls}
      </div>}
      <div
        id={detailsId}
        className='message-error-note__details'
        hidden={!detailsExpanded}
        role='region'
        aria-label={t('conversation.agentError.expandDetails')}
      >
        <ErrorDiagnosticDetails metadata={metadata} unavailable={t('conversation.agentError.details.unavailable')}
          detail={diagnosticDetail} detailLabel={t('conversation.agentError.details.rawDetail')} />
        {feedback && <div className='message-error-note__actions'>
          <FeedbackButton className='message-error-note__feedback' />
        </div>}
      </div>
    </div>
  );
};

export default ConversationErrorNote;
