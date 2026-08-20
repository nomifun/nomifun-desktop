/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Error, History, Loading, Magic, MenuFold, Plus, Robot } from '@icon-park/react';
import { Button, Tooltip } from '@arco-design/web-react';
import React from 'react';

import CreativeStudioAgentComposer from './CreativeStudioAgentComposer';
import CreativeStudioAgentMessages from './CreativeStudioAgentMessages';
import styles from './CreativeStudioAgentPanel.module.css';
import type { CreativeStudioAgentPanelProps } from './types';

const CreativeStudioAgentPanel: React.FC<CreativeStudioAgentPanelProps> = (props) => {
  const panelDisabled = props.disabled === true || props.loadState !== 'ready';

  const renderBody = () => {
    if (props.loadState === 'loading') {
      return (
        <div className={styles.state} data-agent-panel-state='loading' role='status'>
          <Loading className={styles.spin} theme='outline' size='22' />
          <strong>正在加载会话</strong>
          <span>正在恢复当前画布的 Agent 记录</span>
        </div>
      );
    }

    if (props.loadState === 'failed') {
      return (
        <div className={styles.state} data-agent-panel-state='failed' role='alert'>
          <Error theme='outline' size='24' />
          <strong>无法加载 Agent</strong>
          <span>{props.errorMessage ?? '会话加载失败，请重试。'}</span>
          {props.onRetryLoad && (
            <Button size='small' onClick={props.onRetryLoad}>
              重试
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
                onClick={() => props.onSelectSession(session.id)}
              >
                <span className={styles.historyTitle}>{session.title}</span>
                <span className={styles.historyMeta}>
                  {session.messageCount} 条消息
                  {session.updatedAtLabel ? ` · ${session.updatedAtLabel}` : ''}
                </span>
              </button>
            ))
          ) : (
            <div className={styles.historyEmpty}>
              <History theme='outline' size='22' />
              <span>暂无对话记录</span>
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
          <strong>从一个想法开始</strong>
          <span>描述故事、宣传片或现有素材，Agent 会通过 NomiFun 模型与你沟通</span>
        </div>
      );
    }

    return (
      <CreativeStudioAgentMessages
        messages={props.messages}
        onRetryMessage={props.onRetryMessage}
      />
    );
  };

  return (
    <aside
      className={styles.panel}
      data-creative-studio-agent-panel
      data-agent-view={props.view}
      data-agent-running={props.isRunning}
      aria-label='创作 Agent'
    >
      <header className={styles.header}>
        <div className={styles.title}>
          <Robot theme='outline' size='17' />
          <span>{props.view === 'history' ? '历史记录' : '创作 Agent'}</span>
        </div>
        <div className={styles.headerActions}>
          <Tooltip content={props.view === 'history' ? '返回对话' : '历史记录'}>
            <Button
              type='text'
              shape='circle'
              size='small'
              aria-label={props.view === 'history' ? '返回对话' : '查看历史记录'}
              icon={<History theme='outline' size='16' />}
              onClick={() => props.onViewChange(props.view === 'history' ? 'chat' : 'history')}
            />
          </Tooltip>
          <Tooltip content='新对话'>
            <Button
              type='text'
              shape='circle'
              size='small'
              disabled={
                props.activeSessionId === null ||
                props.isRunning ||
                props.loadState !== 'ready'
              }
              aria-label='新对话'
              icon={<Plus theme='outline' size='16' />}
              onClick={() => {
                props.onNewSession();
                props.onViewChange('chat');
              }}
            />
          </Tooltip>
          <Tooltip content='收起 Agent'>
            <Button
              type='text'
              shape='circle'
              size='small'
              aria-label='收起 Agent 面板'
              icon={<MenuFold theme='outline' size='16' />}
              onClick={props.onCollapse}
            />
          </Tooltip>
        </div>
      </header>

      <section className={styles.body}>{renderBody()}</section>

      {props.view === 'chat' && (
        <CreativeStudioAgentComposer
          draft={props.draft}
          model={props.model}
          modelLocked={props.modelLocked === true}
          isRunning={props.isRunning}
          disabled={panelDisabled}
          onDraftChange={props.onDraftChange}
          onModelChange={props.onModelChange}
          onSend={props.onSend}
          onStop={props.onStop}
          onOpenModelSettings={props.onOpenModelSettings}
        />
      )}
    </aside>
  );
};

export default CreativeStudioAgentPanel;
