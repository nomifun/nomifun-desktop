/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Error, History, Loading, Magic, MenuFold, Plus, Robot } from '@icon-park/react';
import { Button, Tooltip } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import CreativeStudioAgentComposer from './CreativeStudioAgentComposer';
import CreativeStudioAgentMessages from './CreativeStudioAgentMessages';
import styles from './CreativeStudioAgentPanel.module.css';
import type { CreativeStudioAgentPanelProps } from './types';

const CreativeStudioAgentPanel: React.FC<CreativeStudioAgentPanelProps> = (props) => {
  const { t } = useTranslation();
  const panelDisabled = props.disabled === true || props.loadState !== 'ready';

  const renderBody = () => {
    if (props.loadState === 'loading') {
      return (
        <div className={styles.state} data-agent-panel-state='loading' role='status'>
          <Loading className={styles.spin} theme='outline' size='22' />
          <strong>
            {t('creativeStudio.agent.loadingTitle', {
              defaultValue: 'Loading conversation',
            })}
          </strong>
          <span>
            {t('creativeStudio.agent.loadingDescription', {
              defaultValue: 'Restoring the Agent history for this canvas',
            })}
          </span>
        </div>
      );
    }

    if (props.loadState === 'failed') {
      return (
        <div className={styles.state} data-agent-panel-state='failed' role='alert'>
          <Error theme='outline' size='24' />
          <strong>
            {t('creativeStudio.agent.loadErrorTitle', {
              defaultValue: 'Could not load Agent',
            })}
          </strong>
          <span>
            {props.errorMessage ??
              t('creativeStudio.agent.loadErrorFallback', {
                defaultValue: 'Conversation loading failed. Try again.',
              })}
          </span>
          {props.onRetryLoad && (
            <Button size='small' onClick={props.onRetryLoad}>
              {t('creativeStudio.agent.retry', { defaultValue: 'Retry' })}
            </Button>
          )}
        </div>
      );
    }

    if (props.view === 'history') {
      return (
        <div className={styles.historyList} data-agent-panel-state='history'>
          {props.sessions.length ? (
            props.sessions.map((session) => (
              <button
                key={session.id}
                type='button'
                className={styles.historyItem}
                data-active={session.id === props.activeSessionId}
                disabled={props.disabled === true || props.isRunning}
                onClick={() => props.onSelectSession(session.id)}
              >
                <span className={styles.historyTitle}>{session.title}</span>
                <span className={styles.historyMeta}>
                  {t('creativeStudio.agent.messageCount', {
                    defaultValue: '{{count}} messages',
                    count: session.messageCount,
                  })}
                  {session.updatedAtLabel ? ` · ${session.updatedAtLabel}` : ''}
                </span>
              </button>
            ))
          ) : (
            <div className={styles.historyEmpty}>
              <History theme='outline' size='22' />
              <span>
                {t('creativeStudio.agent.historyEmpty', {
                  defaultValue: 'No conversation history',
                })}
              </span>
            </div>
          )}
        </div>
      );
    }

    if (!props.messages.length) {
      return (
        <div className={styles.emptyState} data-agent-panel-state='empty'>
          <div className={styles.emptyIcon}>
            <Magic theme='outline' size='21' />
          </div>
          <strong>
            {t('creativeStudio.agent.emptyTitle', {
              defaultValue: 'Start with an idea',
            })}
          </strong>
          <span>
            {t('creativeStudio.agent.emptyDescription', {
              defaultValue:
                'Describe a story, commercial, or existing material and discuss it with the Agent through NomiFun models',
            })}
          </span>
        </div>
      );
    }

    return (
      <CreativeStudioAgentMessages
        messages={props.messages}
        proposals={props.proposals}
        proposalApplyDisabled={props.isRunning || props.disabled === true}
        onRetryMessage={props.onRetryMessage}
        onApplyProposal={props.onApplyProposal}
      />
    );
  };

  return (
    <aside
      className={styles.panel}
      data-creative-studio-agent-panel
      data-agent-view={props.view}
      data-agent-running={props.isRunning}
      aria-label={t('creativeStudio.agent.panelAriaLabel', {
        defaultValue: 'Creative Agent',
      })}
    >
      <header className={styles.header}>
        <div className={styles.title}>
          <Robot theme='outline' size='17' />
          <span>
            {props.view === 'history'
              ? t('creativeStudio.agent.historyTitle', {
                  defaultValue: 'History',
                })
              : t('creativeStudio.agent.title', {
                  defaultValue: 'Creative Agent',
                })}
          </span>
        </div>
        <div className={styles.headerActions}>
          <Tooltip
            content={
              props.view === 'history'
                ? t('creativeStudio.agent.backToChat', {
                    defaultValue: 'Back to conversation',
                  })
                : t('creativeStudio.agent.history', {
                    defaultValue: 'History',
                  })
            }
          >
            <Button
              type='text'
              shape='circle'
              size='small'
              aria-label={
                props.view === 'history'
                  ? t('creativeStudio.agent.backToChat', {
                      defaultValue: 'Back to conversation',
                    })
                  : t('creativeStudio.agent.viewHistory', {
                      defaultValue: 'View history',
                    })
              }
              icon={<History theme='outline' size='16' />}
              onClick={() => props.onViewChange(props.view === 'history' ? 'chat' : 'history')}
            />
          </Tooltip>
          <Tooltip
            content={t('creativeStudio.agent.newConversation', {
              defaultValue: 'New conversation',
            })}
          >
            <Button
              type='text'
              shape='circle'
              size='small'
              disabled={
                props.activeSessionId === null ||
                props.isRunning ||
                props.disabled === true ||
                props.loadState !== 'ready'
              }
              aria-label={t('creativeStudio.agent.newConversation', {
                defaultValue: 'New conversation',
              })}
              icon={<Plus theme='outline' size='16' />}
              onClick={() => {
                props.onNewSession();
                props.onViewChange('chat');
              }}
            />
          </Tooltip>
          <Tooltip
            content={t('creativeStudio.agent.collapse', {
              defaultValue: 'Collapse Agent',
            })}
          >
            <Button
              type='text'
              shape='circle'
              size='small'
              aria-label={t('creativeStudio.agent.collapsePanel', {
                defaultValue: 'Collapse Agent panel',
              })}
              icon={<MenuFold theme='outline' size='16' />}
              onClick={props.onCollapse}
            />
          </Tooltip>
        </div>
      </header>

      <section className={styles.body}>{renderBody()}</section>

      {props.loadState === 'ready' && props.errorMessage ? (
        <div className={styles.inlineError} data-agent-panel-error role='alert'>
          <Error theme='outline' size='15' />
          <span>{props.errorMessage}</span>
        </div>
      ) : null}

      {props.view === 'chat' && (
        <CreativeStudioAgentComposer
          draft={props.draft}
          model={props.model}
          modelLocked={props.modelLocked === true}
          isRunning={props.isRunning}
          disabled={panelDisabled}
          contextItems={props.contextItems}
          skillOptions={props.skillOptions}
          selectedSkillIds={props.selectedSkillIds}
          onDraftChange={props.onDraftChange}
          onModelChange={props.onModelChange}
          onRemoveContextItem={props.onRemoveContextItem}
          onToggleSkill={props.onToggleSkill}
          onSend={props.onSend}
          onStop={props.onStop}
          onOpenModelSettings={props.onOpenModelSettings}
        />
      )}
    </aside>
  );
};

export default CreativeStudioAgentPanel;
