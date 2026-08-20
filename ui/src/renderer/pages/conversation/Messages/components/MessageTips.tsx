/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageTips } from '@/common/chat/chatLib';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { toDisplayText } from '@/common/chat/displayText';
import { Collapse, Message, Tag } from '@arco-design/web-react';
import { Attention, CheckOne } from '@icon-park/react';
import { theme } from '@/platform';
import classNames from 'classnames';
import React, { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import MarkdownView from '@renderer/components/Markdown';
import FeedbackButton from '@renderer/components/base/FeedbackButton';
import CollapsibleContent from '@renderer/components/chat/CollapsibleContent';
import { emitter } from '@/renderer/utils/emitter';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { useMessageList } from '../hooks';
import { parseMessageFileMarker } from './messageFileMarker';
import { MESSAGE_BODY_FONT_SIZE, MESSAGE_BODY_LINE_HEIGHT } from '../typography';

const icon = {
  success: <CheckOne theme='filled' size='16' fill={theme.Color.FunctionalColor.success} className='m-t-2px' />,
  warning: (
    <Attention
      theme='filled'
      size='16'
      strokeLinejoin='bevel'
      className='m-t-2px'
      fill={theme.Color.FunctionalColor.warn}
    />
  ),
  error: (
    <Attention
      theme='filled'
      size='16'
      strokeLinejoin='bevel'
      className='m-t-2px'
      fill={theme.Color.FunctionalColor.error}
    />
  ),
};

const useFormatContent = (content: string) => {
  return useMemo(() => {
    try {
      const json = JSON.parse(content);
      return {
        json: true,
        data: json,
      };
    } catch {
      return { data: content };
    }
  }, [content]);
};

/**
 * Retry entry for a failed turn: recalls the originating user request into
 * the composer via the shared `sendbox.edit` channel (edit mode: submitting
 * truncates and reruns). Only offered on the nomi surface, for errors that
 * answer the latest user request, once the turn has settled.
 */
const useErrorRetry = (message: IMessageTips): (() => void) | null => {
  const conversationContext = useConversationContextSafe();
  const messageList = useMessageList();
  return useMemo(() => {
    if (message.content.type !== 'error') return null;
    if (message.content.recovery) return null;
    if (message.content.error?.retryable === false) return null;
    if (conversationContext?.type !== 'nomi') return null;
    if (conversationContext.readOnly === true) return null;
    if (conversationContext.isProcessing === true) return null;
    const lastRight = messageList.findLast((entry) => entry.type === 'text' && entry.position === 'right');
    if (!lastRight || lastRight.type !== 'text') return null;
    const retryMessageId = lastRight.message_id ?? lastRight.msg_id;
    const retryCreatedAt = lastRight.created_at;
    if (!retryMessageId || retryCreatedAt == null) return null;
    if ((message.created_at ?? 0) < retryCreatedAt) return null;
    const rawContent = typeof lastRight.content?.content === 'string' ? lastRight.content.content : '';
    const { text } = parseMessageFileMarker(rawContent, 'right');
    if (!text.trim()) return null;
    return () => emitter.emit('sendbox.edit', { msgId: retryMessageId, createdAt: retryCreatedAt, content: text });
  }, [conversationContext, message.content, message.created_at, messageList]);
};

type ContinueState = 'idle' | 'pending' | 'accepted' | 'stale';

const useTruncatedContinuation = (message: IMessageTips) => {
  const { t } = useTranslation();
  const conversationContext = useConversationContextSafe();
  const [state, setState] = useState<ContinueState>('idle');
  const recovery = message.content.recovery;
  const expectedUiErrorCode =
    recovery?.failure_code === 'output_truncated'
      ? 'OUTPUT_TRUNCATED'
      : recovery?.failure_code === 'turn_requests_exhausted'
        ? 'TURN_REQUESTS_EXHAUSTED'
        : undefined;
  const visible = Boolean(
    message.content.type === 'error' &&
      message.content.error?.retryable === true &&
      recovery &&
      message.content.error.code === expectedUiErrorCode &&
      conversationContext?.type === 'nomi' &&
      conversationContext.readOnly !== true
  );
  const disabled =
    !visible || conversationContext?.isProcessing === true || state === 'pending' || state === 'accepted' || state === 'stale';

  const continueTurn = useCallback(async () => {
    if (!recovery || !conversationContext || disabled) return;
    setState('pending');
    try {
      await ipcBridge.conversation.continueTruncated.invoke({
        conversation_id: conversationContext.conversation_id,
        source_message_id: recovery.source_message_id,
        // One source failure owns exactly one continuation operation. The
        // stable key absorbs double-clicks, transport retries, and remounts.
        idempotency_key: recovery.source_message_id,
      });
      setState('accepted');
    } catch (error) {
      if (isBackendHttpError(error) && error.status === 409) {
        setState('stale');
        Message.warning(
          t('conversation.truncation.stale', {
            defaultValue: 'This interrupted turn has already been superseded.',
          })
        );
        return;
      }
      setState('idle');
      Message.error(
        t('conversation.truncation.failed', {
          defaultValue: 'Could not continue the interrupted turn.',
        })
      );
    }
  }, [conversationContext, disabled, recovery, t]);

  const label =
    state === 'pending'
      ? t('conversation.truncation.continuing', { defaultValue: 'Continuing…' })
      : state === 'accepted'
        ? t('conversation.truncation.accepted', { defaultValue: 'Continuation started' })
        : state === 'stale'
          ? t('conversation.truncation.superseded', { defaultValue: 'Already superseded' })
          : t('conversation.truncation.continue', { defaultValue: 'Continue execution' });

  return { visible, disabled, label, continueTurn };
};

const MessageTips: React.FC<{ message: IMessageTips }> = ({ message }) => {
  const { t } = useTranslation();
  const { type } = message.content;
  const content = toDisplayText(message.content.content);
  const structuredError = type === 'error' ? message.content.error : undefined;
  const { json, data } = useFormatContent(content);
  const retry = useErrorRetry(message);
  const continuation = useTruncatedContinuation(message);
  const retryButton = retry ? (
    <button type='button' className='message-error-note__retry' data-testid='message-error-retry' onClick={retry}>
      {t('common.retry', { defaultValue: 'Retry' })}
    </button>
  ) : null;
  const continueButton = continuation.visible ? (
    <button
      type='button'
      className='message-error-note__retry'
      data-testid='message-error-continue-truncated'
      disabled={continuation.disabled}
      onClick={() => void continuation.continueTurn()}
    >
      {continuation.label}
    </button>
  ) : null;
  const recoveryButton = continueButton ?? retryButton;

  const displayContent = json ? '' : content;
  const shouldShowFeedback = type === 'error';

  if (structuredError) {
    const code = structuredError.code;
    const ownership = structuredError.ownership;
    const title = code
      ? t(`conversation.agentError.codes.${code}.title`, {
          defaultValue: t('conversation.agentError.fallbackTitle'),
        })
      : t('conversation.agentError.fallbackTitle');
    const body = code
      ? t(
          structuredError.workspacePath
            ? `conversation.agentError.codes.${code}.bodyWithPath`
            : `conversation.agentError.codes.${code}.body`,
          {
            workspacePath: structuredError.workspacePath,
            defaultValue: structuredError.message || content,
          }
        )
      : structuredError.message || content;
    const ownershipLabel = ownership
      ? t(`conversation.agentError.ownership.${ownership}`, {
          defaultValue: t('conversation.agentError.ownership.unknown_upstream'),
        })
      : null;
    const retryHint =
      structuredError.retryable === undefined
        ? null
        : structuredError.retryable
          ? t('conversation.agentError.retryable')
          : t('conversation.agentError.notRetryable');
    const resolutionText = structuredError.resolution
      ? t(`conversation.agentError.resolution.${structuredError.resolution.kind}`)
      : null;
    const detailParts = [
      code ? `${t('conversation.agentError.errorCode')}: ${code}` : '',
      structuredError.detail || structuredError.message,
    ].filter(Boolean);

    return (
      <div className='w-full'>
        <div className={classNames('message-error-note', ownership && `message-error-note--${ownership}`)}>
          <div className='message-error-note__rail' aria-hidden='true' />
          <div className='message-error-note__content'>
            <div className='message-error-note__header'>
              <div className='message-error-note__status'>
                <span className='message-error-note__icon'>{icon.error}</span>
                {ownershipLabel && <span className='message-error-note__owner'>{ownershipLabel}</span>}
              </div>
              <div className='message-error-note__meta'>
                {retryHint && (
                  <Tag
                    size='small'
                    color={structuredError.retryable ? 'green' : 'gray'}
                    className='message-error-note__tag'
                  >
                    {retryHint}
                  </Tag>
                )}
                {code && <span className='message-error-note__code'>{code}</span>}
              </div>
            </div>
            <div className='message-error-note__main'>
              <div className='message-error-note__title'>{title}</div>
              <div className='message-error-note__body'>{body}</div>
              {resolutionText && (
                <div className='message-error-note__resolution'>
                  <span className='message-error-note__resolution-label'>
                    {t('conversation.agentError.resolutionPrefix')}
                  </span>
                  <span>{resolutionText}</span>
                </div>
              )}
              <div className='message-error-note__footer'>
                <div className='message-error-note__footer-main'>
                  {detailParts.length > 0 && (
                    <Collapse bordered={false} className='message-error-note__details'>
                      <Collapse.Item
                        name='technical-details'
                        header={
                          <span className='message-error-note__details-label'>{t('common.technical_details')}</span>
                        }
                      >
                        <div className='message-error-note__detail-body'>{detailParts.join('\n')}</div>
                      </Collapse.Item>
                    </Collapse>
                  )}
                  {shouldShowFeedback && (
                    <div className='message-error-note__actions'>
                      {recoveryButton}
                      <FeedbackButton className='message-error-note__feedback' />
                    </div>
                  )}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    );
  }

  if (json)
    return (
      <div className='w-full'>
        <div className={classNames('bg-message-tips rd-8px p-x-12px p-y-8px flex flex-col gap-4px')}>
          <div className='flex items-start gap-4px'>
            {icon[type] || icon.warning}
            <div className='flex-1 min-w-0'>
              <MarkdownView fontSize={MESSAGE_BODY_FONT_SIZE} lineHeight={MESSAGE_BODY_LINE_HEIGHT}>
                {`\`\`\`json\n${JSON.stringify(data, null, 2)}\n\`\`\``}
              </MarkdownView>
            </div>
          </div>
          {type === 'error' && (
            <div className='flex justify-end items-center gap-8px'>
              {recoveryButton}
              <FeedbackButton />
            </div>
          )}
        </div>
      </div>
    );
  return (
    <div className='w-full'>
      <div className={classNames('bg-message-tips rd-8px  p-x-12px p-y-8px flex flex-col gap-4px')}>
        <div className='flex items-start gap-4px'>
          {icon[type] || icon.warning}
          <div className='flex-1 min-w-0'>
            <CollapsibleContent maxHeight={48} defaultCollapsed={true} useMask={true}>
              <span className='whitespace-break-spaces text-t-primary [word-break:break-word]'>{displayContent}</span>
            </CollapsibleContent>
          </div>
        </div>
        {shouldShowFeedback && (
          <div className='flex justify-end items-center gap-8px'>
            {recoveryButton}
            <FeedbackButton />
          </div>
        )}
      </div>
    </div>
  );
};

export default MessageTips;
