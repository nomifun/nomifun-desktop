/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import ThinkingProcessDisplay from '@renderer/components/chat/ThinkingProcessDisplay';
import { CheckOne, Error, MagicWand, Refresh, Robot } from '@icon-park/react';
import { Button } from '@arco-design/web-react';
import React from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import type {
  CreativeStudioAgentMessage,
  CreativeStudioAgentProposal,
} from './types';
import styles from './CreativeStudioAgentPanel.module.css';

interface CreativeStudioAgentMessagesProps {
  messages: readonly CreativeStudioAgentMessage[];
  proposals: readonly CreativeStudioAgentProposal[];
  proposalApplyDisabled: boolean;
  onRetryMessage?(messageId: string): void;
  onApplyProposal(messageId: string): void;
}

const CreativeStudioAgentMessages: React.FC<CreativeStudioAgentMessagesProps> = ({
  messages,
  proposals,
  proposalApplyDisabled,
  onRetryMessage,
  onApplyProposal,
}) => (
  <div className={styles.messageList} aria-live='polite'>
    {messages.map((message) => {
      const isAssistant = message.role === 'assistant';
      const proposal = proposals.find((candidate) => candidate.messageId === message.id);
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
            <ThinkingProcessDisplay
              className={styles.thinkingStatus}
              state='running'
              subject={message.activityLabel}
              identityKey={message.id}
              disclosure={false}
              runningFallbackLabel='Agent 正在工作'
              role='status'
            />
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

          {proposal ? (
            <section
              className={styles.proposalCard}
              data-agent-proposal-state={proposal.state}
              aria-label='画布操作提案'
              role={proposal.state === 'failed' || proposal.state === 'invalid' ? 'alert' : 'group'}
            >
              <div className={styles.proposalHeading}>
                {proposal.state === 'applied' ? (
                  <CheckOne theme='filled' size='16' />
                ) : (
                  <MagicWand theme='outline' size='16' />
                )}
                <div>
                  <strong>{proposal.summary}</strong>
                  <span>{proposal.opCount} 项画布操作 · 需人工确认</span>
                </div>
              </div>
              {proposal.errorMessage ? (
                <p className={styles.proposalError}>{proposal.errorMessage}</p>
              ) : null}
              <Button
                size='small'
                type={proposal.state === 'ready' ? 'primary' : 'secondary'}
                loading={proposal.state === 'applying'}
                disabled={proposal.state !== 'ready' || proposalApplyDisabled}
                onClick={() => onApplyProposal(proposal.messageId)}
              >
                {proposal.state === 'ready'
                  ? '应用到画布'
                  : proposal.state === 'applying'
                    ? '正在应用'
                    : proposal.state === 'applied'
                      ? '已应用'
                      : '不可应用'}
              </Button>
            </section>
          ) : null}

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
