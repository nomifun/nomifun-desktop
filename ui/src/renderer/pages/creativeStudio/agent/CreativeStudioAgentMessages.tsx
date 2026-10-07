/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import ThinkingProcessDisplay from '@renderer/components/chat/ThinkingProcessDisplay';
import ConversationErrorNote from '@/renderer/pages/conversation/Messages/components/ConversationErrorNote';
import { conversationPauseError } from '@/renderer/pages/conversation/utils/conversationPauseError';
import { CheckOne, MagicWand, Refresh, Robot } from '@icon-park/react';
import { Button } from '@arco-design/web-react';
import React from 'react';
import ReactMarkdown from 'react-markdown';
import { useTranslation } from 'react-i18next';
import remarkGfm from 'remark-gfm';

import type {
  CreativeStudioAgentMessage,
  CreativeStudioAgentProposal,
} from './types';
import styles from './CreativeStudioAgentPanel.module.css';

interface CreativeStudioAgentMessagesProps {
  conversationId?: string;
  messages: readonly CreativeStudioAgentMessage[];
  proposals: readonly CreativeStudioAgentProposal[];
  proposalApplyDisabled: boolean;
  onRetryMessage?(messageId: string): void;
  onApplyProposal(messageId: string): void;
}

const CreativeStudioAgentMessages: React.FC<CreativeStudioAgentMessagesProps> = ({
  conversationId,
  messages,
  proposals,
  proposalApplyDisabled,
  onRetryMessage,
  onApplyProposal,
}) => {
  const { t } = useTranslation();

  return (
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
                    <span>{t('creativeStudio.agent.name', { defaultValue: 'Agent' })}</span>
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
                runningFallbackLabel={t('creativeStudio.agent.runningFallback', {
                  defaultValue: 'Agent is working',
                })}
                role='status'
              />
            )}

            {message.status === 'failed' && (
              <ConversationErrorNote
                error={message.error ?? { message: message.errorMessage, code: 'CONVERSATION_SEND_FAILED' }}
                rawDetail={message.error?.detail || message.errorMessage}
                timestamp={message.timestamp}
                turnId={message.turnId}
                sessionId={conversationId}
                feedback={false}
                recoveryAction={message.id === messages.at(-1)?.id && onRetryMessage && message.error?.retryable !== false ? (
                  <button type='button' className='message-error-note__retry'
                    aria-label={t('creativeStudio.agent.retryMessage', { defaultValue: 'Retry this message' })}
                    onClick={() => onRetryMessage(message.id)}>
                    <Refresh theme='outline' size='14' aria-hidden='true' />
                    {t('creativeStudio.agent.retryMessage', { defaultValue: 'Retry this message' })}
                  </button>
                ) : null}
              />
            )}

            {message.status === 'paused' && <ConversationErrorNote
              error={conversationPauseError(message.pause)}
              timestamp={message.pause.pausedAt}
              turnId={message.pause.turnId}
              sessionId={conversationId}
              feedback={false}
            />}

            {message.status === 'stopped' && (
              <div className={styles.stoppedLabel} role='status'>
                {t('creativeStudio.agent.stopped', { defaultValue: 'Stopped' })}
              </div>
            )}

            {proposal ? (
              <section
                className={styles.proposalCard}
                data-agent-proposal-state={proposal.state}
                aria-label={t('creativeStudio.agent.proposal.ariaLabel', {
                  defaultValue: 'Canvas operation proposal',
                })}
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
                    <span>
                      {t('creativeStudio.agent.proposal.operationSummary', {
                        defaultValue: '{{count}} canvas operations · requires manual confirmation',
                        count: proposal.opCount,
                      })}
                    </span>
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
                    ? t('creativeStudio.agent.proposal.apply', {
                        defaultValue: 'Apply to canvas',
                      })
                    : proposal.state === 'applying'
                      ? t('creativeStudio.agent.proposal.applying', {
                          defaultValue: 'Applying',
                        })
                      : proposal.state === 'applied'
                        ? t('creativeStudio.agent.proposal.applied', {
                            defaultValue: 'Applied',
                          })
                        : t('creativeStudio.agent.proposal.unavailable', {
                            defaultValue: 'Unavailable',
                          })}
                </Button>
              </section>
            ) : null}

          </article>
        );
      })}
    </div>
  );
};

export default CreativeStudioAgentMessages;
