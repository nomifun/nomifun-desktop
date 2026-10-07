/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageTips } from '@/common/chat/chatLib';
import { toDisplayText } from '@/common/chat/displayText';
import { Attention, CheckOne, Refresh } from '@icon-park/react';
import { theme } from '@/platform';
import classNames from 'classnames';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import MarkdownView from '@renderer/components/Markdown';
import CollapsibleContent from '@renderer/components/chat/CollapsibleContent';
import { emitter } from '@/renderer/utils/emitter';
import { useConversationContextSafe } from '@/renderer/hooks/context/ConversationContext';
import { useMessageList } from '../hooks';
import { parseMessageFileMarker } from './messageFileMarker';
import { MESSAGE_BODY_FONT_SIZE, MESSAGE_BODY_LINE_HEIGHT } from '../typography';
import { TEMPLATE_I18N_PATH } from '@/renderer/pages/agentSettings/model';
import { IdmmDecisionNotice } from './IdmmDecisionNotice';
import ConversationErrorNote from './ConversationErrorNote';

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
 * the composer via the shared `sendbox.edit` channel. Submitting the recalled
 * request creates a new canonical Turn and never mutates history. Only offered
 * on the nomi surface, for errors that
 * answer the latest user request, once the turn has settled.
 */
const useErrorRetry = (message: IMessageTips): (() => void) | null => {
  const conversationContext = useConversationContextSafe();
  const messageList = useMessageList();
  return useMemo(() => {
    if (message.content.type !== 'error') return null;
    if (message.content.idmm_notice) return null;
    if (message.content.recovery || message.content.execution_pause || conversationContext?.executionPause) return null;
    if (message.content.error?.retryable === false) return null;
    if (conversationContext?.type !== 'nomi') return null;
    if (conversationContext.readOnly === true) return null;
    if (conversationContext.isProcessing === true) return null;
    const lastRight = messageList.findLast((entry) => entry.type === 'text' && entry.position === 'right');
    if (!lastRight || lastRight.type !== 'text') return null;
    if (lastRight.content.idmm_decision) return null;
    const retryMessageId = lastRight.message_id ?? lastRight.msg_id;
    const retryCreatedAt = lastRight.created_at;
    if (!retryMessageId || retryCreatedAt == null) return null;
    if (message.turn_id && message.turn_id !== retryMessageId) return null;
    if ((message.created_at ?? 0) < retryCreatedAt) return null;
    const rawContent = typeof lastRight.content?.content === 'string' ? lastRight.content.content : '';
    const { text } = parseMessageFileMarker(rawContent, 'right');
    if (!text.trim()) return null;
    return () => emitter.emit('sendbox.edit', { msgId: retryMessageId, createdAt: retryCreatedAt, content: text });
  }, [conversationContext, message.content, message.created_at, message.turn_id, messageList]);
};

const MessageTips: React.FC<{ message: IMessageTips }> = ({ message }) => {
  const { t } = useTranslation();
  const conversationContext = useConversationContextSafe();
  const currentAgent = conversationContext?.currentAgent;
  const { type } = message.content;
  const transition = message.content.agent_transition;
  const content = transition
    ? t('conversation.chat.agentSwitch.transitionMarker', {
        from: transition.previous_template_key
          ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[transition.previous_template_key]}.name`)
          : currentAgent && currentAgent.presetId === transition.previous_preset_id
            ? currentAgent.label : transition.previous_agent_label,
        to: transition.next_template_key
          ? t(`agentSettings.template.${TEMPLATE_I18N_PATH[transition.next_template_key]}.name`)
          : currentAgent && currentAgent.presetId === transition.next_preset_id
            ? currentAgent.label : transition.next_agent_label,
      })
    : toDisplayText(message.content.content);
  const structuredError = type === 'error' ? message.content.error : undefined;
  const { json, data } = useFormatContent(content);
  const retry = useErrorRetry(message);
  const retryButton = retry ? (
    <button type='button' className='message-error-note__retry' data-testid='message-error-retry' onClick={retry}>
      <Refresh theme='outline' size='14' fill='currentColor' aria-hidden='true' />
      {t('common.retry', { defaultValue: 'Retry' })}
    </button>
  ) : null;

  const displayContent = json ? '' : content;
  if (message.content.idmm_notice) return <IdmmDecisionNotice message={message} />;
  if (transition) {
    return (
      <div className='agent-transition-boundary' role='note' aria-label={content}>
        <span className='agent-transition-boundary__rule' aria-hidden='true' />
        <span className='agent-transition-boundary__label'>{content}</span>
        <span className='agent-transition-boundary__rule' aria-hidden='true' />
      </div>
    );
  }
  if (type !== 'error' && !content.trim()) return null;
  if (type === 'error') {
    return <ConversationErrorNote
      error={structuredError}
      rawDetail={structuredError?.detail || structuredError?.message || (json ? JSON.stringify(data, null, 2) : content)}
      timestamp={message.content.finished_at_ms ?? message.created_at}
      turnId={message.turn_id}
      sessionId={message.conversation_id}
      recoveryAction={retryButton}
      currentModel={conversationContext?.currentModel}
    />;
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
      </div>
    </div>
  );
};

export default MessageTips;
