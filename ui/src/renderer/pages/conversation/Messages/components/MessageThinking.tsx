/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IMessageThinking } from '@/common/chat/chatLib';
import { toDisplayText } from '@/common/chat/displayText';
import ThinkingProcessDisplay from '@renderer/components/chat/ThinkingProcessDisplay';
import React from 'react';
import { useTranslation } from 'react-i18next';

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

  const formatElapsedTime = (seconds: number): string => {
    const sUnit = t('common.unit.second_short', { defaultValue: 's' });
    const mUnit = t('common.unit.minute_short', { defaultValue: 'm' });

    if (seconds < 60) return `${seconds}${sUnit}`;
    const minutes = Math.floor(seconds / 60);
    const remaining = seconds % 60;
    return `${minutes}${mUnit} ${remaining}${sUnit}`;
  };

  const { status, subject } = message.content;
  const text = toDisplayText(message.content.content);
  const isDone = completed === true || status === 'done';

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
      formatElapsedTime={formatElapsedTime}
    />
  );
};

export default MessageThinking;
