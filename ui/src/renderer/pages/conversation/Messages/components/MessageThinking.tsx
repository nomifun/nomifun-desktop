/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageThinking } from '@/common/chat/chatLib';
import { buildCompletedThinkingSummary } from '@/common/config/thinkingDisplay';
import { toDisplayText } from '@/common/chat/displayText';
import ThinkingProcessDisplay from '@renderer/components/chat/ThinkingProcessDisplay';
import MarkdownView from '@renderer/components/Markdown';
import { useThinkingDisplayPreferences } from '@renderer/hooks/config/useThinkingDisplayPreferences';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { MESSAGE_BODY_FONT_SIZE, MESSAGE_BODY_LINE_HEIGHT } from '../typography';

interface MessageThinkingProps {
  message: IMessageThinking;
  variant?: 'standalone' | 'process';
  completed?: boolean;
  expanded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
}

const MessageThinking: React.FC<MessageThinkingProps> = ({
  message,
  variant = 'standalone',
  completed,
  expanded,
  onExpandedChange,
}) => {
  const { t } = useTranslation();
  const thinkingDisplay = useThinkingDisplayPreferences();

  const { status, subject } = message.content;
  const text = toDisplayText(message.content.content);
  const isDone = completed === true || status === 'done';
  const completedSummary = isDone
    ? buildCompletedThinkingSummary(toDisplayText(subject), text, thinkingDisplay.summaryLength)
    : '';

  if (!thinkingDisplay.visible) return null;

  return (
    <ThinkingProcessDisplay
      state={isDone ? 'completed' : 'running'}
      subject={toDisplayText(subject)}
      content={text}
      startedAt={message.created_at}
      identityKey={message.msg_id ?? message.id}
      variant={variant}
      expanded={expanded}
      onExpandedChange={onExpandedChange}
      runningFallbackLabel={t('conversation.thinking.label', {
        defaultValue: 'Thinking...',
      })}
      completedLabel={t('conversation.thinking.complete', {
        defaultValue: 'Thought complete',
      })}
      completedSummary={completedSummary}
      bodyLength={thinkingDisplay.contentLength}
      showElapsedTime={false}
    >
      <MarkdownView fontSize={MESSAGE_BODY_FONT_SIZE} lineHeight={MESSAGE_BODY_LINE_HEIGHT}>
        {text}
      </MarkdownView>
    </ThinkingProcessDisplay>
  );
};

export default MessageThinking;
