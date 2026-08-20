/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Error, Loading, Refresh, Robot } from '@icon-park/react';
import { Button } from '@arco-design/web-react';
import React from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import type { CreativeStudioAgentMessage } from './types';
import styles from './CreativeStudioAgentPanel.module.css';

interface CreativeStudioAgentMessagesProps {
  messages: readonly CreativeStudioAgentMessage[];
  onRetryMessage?(messageId: string): void;
}

const CreativeStudioAgentMessages: React.FC<CreativeStudioAgentMessagesProps> = ({
  messages,
  onRetryMessage,
}) => (
  <div className={styles.messageList} aria-live='polite'>
    {messages.map((message) => {
      const isAssistant = message.role === 'assistant';
      return (
        <article
          key={message.id}
          className={isAssistant ? styles.assistantMessage : styles.userMessage}
          data-agent-message-role={message.role}
          data-agent-message-status={message.status}
        >
          {message.text ? (
            <div className={styles.messageBubble}>
              {isAssistant && (
                <div className={styles.agentIdentity}>
                  <Robot theme='outline' size='14' />
                  <span>Agent</span>
                </div>
              )}
              {isAssistant ? (
                <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml>
                  {message.text}
                </ReactMarkdown>
              ) : (
                <span className={styles.userText}>{message.text}</span>
              )}
            </div>
          ) : null}

          {message.status === 'running' && (
            <div className={styles.activityCard} role='status'>
              <Loading className={styles.spin} theme='outline' size='16' />
              <span>{message.activityLabel ?? 'Agent 正在工作'}</span>
            </div>
          )}

          {message.status === 'failed' && (
            <div className={styles.messageError} role='alert'>
              <Error theme='outline' size='15' />
              <span>{message.errorMessage}</span>
            </div>
          )}

          {message.status === 'stopped' && (
            <div className={styles.stoppedLabel} role='status'>
              已停止
            </div>
          )}

          {isAssistant && message.status !== 'running' && onRetryMessage && (
            <Button
              className={styles.retryMessageButton}
              type='text'
              shape='circle'
              size='mini'
              aria-label='重试这条消息'
              icon={<Refresh theme='outline' size='14' />}
              onClick={() => onRetryMessage(message.id)}
            />
          )}
        </article>
      );
    })}
  </div>
);

export default CreativeStudioAgentMessages;
