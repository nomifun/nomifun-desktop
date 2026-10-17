/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import useSWR from 'swr';
import { Button, Spin } from '@arco-design/web-react';
import { BookOne, Connection, Data, Right, RobotOne, SettingTwo } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { CompanionId } from '@/common/types/ids';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { ExecutionProvider } from '@/renderer/pages/conversation/execution/ExecutionContext';
import ExecutionContentSwitcher from '@/renderer/pages/conversation/execution/ExecutionContentSwitcher';
import { PreviewProvider } from '@/renderer/pages/conversation/Preview';
import { browserStorageKey } from '@/common/utils/browserStorageKey';
import CompanionConversation from '../companion/CompanionConversation';
import CompanionAppearanceQuickSettings from './CompanionAppearanceQuickSettings';
import CompanionAgentModelQuickSettings from './CompanionAgentModelQuickSettings';
import CompanionRobotResourceSettings from './CompanionRobotResourceSettings';
import { useCompanionConversationUpdates } from './useCompanionConversationUpdates';
import { useCompanionSessions } from './useCompanionSessions';
import CompanionSessionToolbar from './CompanionSessionToolbar';
import type { CompanionHandle, WorkspaceTabKey } from './types';
import { isConversationBindingBlocked } from '@/renderer/pages/conversation/utils/conversationRuntime';
import styles from './CompanionCohabitView.module.css';

type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

interface Props {
  companionId: CompanionId;
  companion: CompanionHandle;
  onManage: (tab: WorkspaceTabKey) => void;
  infoPanelCollapsed: boolean;
}

const CompanionCohabitView: React.FC<Props> = ({ companionId, companion, onManage, infoPanelCollapsed }) => {
  const { t } = useTranslation();
  const { profile, status } = companion;
  const sessions = useCompanionSessions(companionId, Boolean(profile?.model));
  const { conversationId } = sessions;
  const [robotCount, setRobotCount] = useState(0);
  const [appearanceSaving, setAppearanceSaving] = useState(false);
  const [agentModelSaving, setAgentModelSaving] = useState(false);
  const [robotResourceSaving, setRobotResourceSaving] = useState(false);
  const quickSettingsSaving = appearanceSaving || agentModelSaving || robotResourceSaving;
  const activeCompanionId = useRef(companionId);
  activeCompanionId.current = companionId;
  const onAppearanceSavingChange = useCallback((saving: boolean) => {
    if (activeCompanionId.current === companionId) setAppearanceSaving(saving);
  }, [companionId]);
  const onAgentModelSavingChange = useCallback((saving: boolean) => {
    if (activeCompanionId.current === companionId) setAgentModelSaving(saving);
  }, [companionId]);
  const onRobotResourceSavingChange = useCallback((saving: boolean) => {
    if (activeCompanionId.current === companionId) setRobotResourceSaving(saving);
  }, [companionId]);

  useEffect(() => {
    setAppearanceSaving(false);
    setAgentModelSaving(false);
    setRobotResourceSaving(false);
  }, [companionId]);

  useEffect(() => {
    let cancelled = false;
    void ipcBridge.robot.list.invoke()
      .then((robots) => {
        if (!cancelled) setRobotCount(robots.filter((robot) => robot.companion_id === companionId).length);
      })
      .catch(() => {
        if (!cancelled) setRobotCount(0);
      });
    return () => {
      cancelled = true;
    };
  }, [companionId]);

  const { data: loadedConversation, error: conversationError, isLoading, mutate } = useSWR<NomiConversation | null>(
    conversationId ? `conversation/${conversationId}` : null,
    async () => {
      const row = await getConversationOrNull(conversationId!);
      if (!row || row.type !== 'nomi' || row.extra.companion_id !== companionId
        || !['companion', 'robot'].includes(row.extra.companion_role ?? '')) {
        throw new Error('Session does not belong to this companion');
      }
      return row;
    }
  );
  const conversation = loadedConversation?.id === conversationId && loadedConversation.extra.companion_id === companionId
    && ['companion', 'robot'].includes(loadedConversation.extra.companion_role ?? '') ? loadedConversation : undefined;

  useCompanionConversationUpdates(conversationId, mutate);
  const sessionError = sessions.error || (sessions.missing && !conversation) || Boolean(conversationError)
    || Boolean(loadedConversation && loadedConversation.id === conversationId && !conversation);
  const role = conversation?.extra.companion_role;
  const refreshConversation = useCallback(() => {
    void mutate().catch(() => {});
  }, [mutate]);

  const memoryCount = (status?.memories_active ?? 0) + (status?.memories_archived ?? 0);
  const growthBase = status ? Math.max(0, (status.level - 1) ** 2 * 100) : 0;
  const growthNext = status ? Math.max(100, status.level ** 2 * 100) : 100;
  const growthPercent = status
    ? Math.max(0, Math.min(100, Math.round(((status.xp - growthBase) / (growthNext - growthBase)) * 100)))
    : 0;

  const chat = useMemo(() => {
    if (sessions.selectionCleared) return <div className={styles.chatState}>
      <strong>{t('nomi.cohabit.sessions.chooseOrCreate')}</strong>
    </div>;
    if (!conversationId && !profile?.model) {
      return (
        <div className={styles.chatState}>
          <RobotOne theme='outline' size='30' fill='currentColor' />
          <strong>{t('nomi.cohabit.modelRequired', { defaultValue: '先为伙伴配置主对话模型' })}</strong>
          <span>{t('nomi.cohabit.modelRequiredHint', { defaultValue: '配置伙伴默认模型后即可新建桌面会话；机器人使用独立设置。' })}</span>
          <Button type='primary' shape='round' onClick={() => onManage('overview')}>
            {t('nomi.cohabit.configureModel', { defaultValue: '配置模型' })}
          </Button>
        </div>
      );
    }
    if (sessionError) {
      return (
        <div className={styles.chatState}>
          <strong>{t('nomi.cohabit.sessionFailed', { defaultValue: '伙伴会话暂时无法打开' })}</strong>
          <span>{t('nomi.cohabit.sessionFailedHint', { defaultValue: '检查模型与 Agent 配置后再试一次。' })}</span>
          <Button shape='round' onClick={() => onManage('overview')}>
            {t('nomi.workspace.manage', { defaultValue: '进入管理' })}
          </Button>
        </div>
      );
    }
    if (!conversationId || isLoading || !conversation) {
      return <div className={styles.chatState}><Spin /></div>;
    }
    return (
      <PreviewProvider
        persistNamespace={browserStorageKey('workspace-preview', 'conversation', conversation.id)}
        subscribeGlobalOpen
      >
        <ExecutionProvider key={conversation.id} conversation={conversation}>
          <ExecutionContentSwitcher>
            <CompanionConversation key={conversation.id} conversation={conversation} companion={companion} compact />
          </ExecutionContentSwitcher>
        </ExecutionProvider>
      </PreviewProvider>
    );
  }, [companion, conversation, conversationId, isLoading, onManage, profile?.model, sessionError, sessions.selectionCleared, t]);

  return (
    <div className={styles.layout} data-info-panel-collapsed={infoPanelCollapsed}>
      <section className={styles.chatColumn}>
        <CompanionSessionToolbar items={sessions.items} selectedId={conversationId} selectedName={conversation?.name}
          selectedLoaded={Boolean(conversation)}
          role={role} activeDesktopId={sessions.activeDesktopId} creating={sessions.creating} routing={sessions.routing}
          selectedArchived={conversation?.extra.canonical_archived} managing={sessions.managing}
          clearDisabled={!conversation || isConversationBindingBlocked(conversation)}
          canCreate={Boolean(profile?.model)} failed={sessions.actionError || sessions.listError}
          onSelect={sessions.select} onCreate={() => void sessions.create()} onContinue={() => void sessions.continueInWindow()}
          onRefresh={sessions.refresh} onRename={sessions.rename} onArchive={sessions.setArchived}
          onDelete={sessions.remove} onClear={sessions.clearContext} />
        <div className={styles.chatBody}>{chat}</div>
      </section>

      {!infoPanelCollapsed && <aside className={styles.info} aria-label={t('nomi.cohabit.infoTitle', { defaultValue: '伙伴信息' })}>
        <div className={styles.infoHeader}>
          <h2>{t('nomi.cohabit.infoTitle', { defaultValue: '伙伴信息' })}</h2>
          <button type='button' onClick={() => onManage('overview')}>
            {t('nomi.workspace.manage', { defaultValue: '管理' })}<Right size='13' />
          </button>
        </div>

        <section className={styles.infoGroup}>
          <div className={styles.infoTitle}><SettingTwo size='16' />{t('nomi.cohabit.currentState', { defaultValue: '当前状态' })}</div>
          <CompanionAppearanceQuickSettings key={companionId} companion={companion} disabled={quickSettingsSaving} onSavingChange={onAppearanceSavingChange} />
        </section>

        <section className={styles.infoGroup}>
          <div className={styles.infoTitle}><RobotOne size='16' />{t('nomi.cohabit.sessions.companion')}</div>
          <CompanionAgentModelQuickSettings key={`${companionId}/companion`} companion={companion} role='companion'
            conversation={role === 'companion' ? conversation : undefined} disabled={quickSettingsSaving || Boolean(conversationError)} onChanged={refreshConversation} onSavingChange={onAgentModelSavingChange} />
          {conversationError && <span role='alert'>
            {t('agentSettings.productBinding.loadFailed')}
            <button type='button' onClick={refreshConversation}>{t('agentSettings.actions.retry')}</button>
          </span>}
        </section>

        <section className={styles.infoGroup}>
          <div className={styles.infoTitle}><RobotOne size='16' />{t('nomi.cohabit.sessions.robot')}</div>
          <CompanionAgentModelQuickSettings key={`${companionId}/robot`} companion={companion} role='robot'
            conversation={role === 'robot' ? conversation : undefined} disabled={quickSettingsSaving || Boolean(conversationError)} onChanged={refreshConversation} onSavingChange={onAgentModelSavingChange} />
        </section>

        <section className={styles.infoGroup}>
          <div className={styles.infoTitle}><Data size='16' />{t('nomi.cohabit.longMemory', { defaultValue: '长期记忆' })}</div>
          <strong>{t('nomi.cohabit.memoryCount', { count: memoryCount, defaultValue: '{{count}} 条记忆' })}</strong>
          <span>{t('nomi.cohabit.knowledgeReady', { defaultValue: '专属知识库按需挂载' })}</span>
          <button type='button' onClick={() => onManage('memory')}>{t('nomi.cohabit.manageMemory', { defaultValue: '查看记忆与知识' })}</button>
        </section>

        <section className={styles.infoGroup}>
          <div className={styles.infoTitle}><Connection size='16' />{t('nomi.cohabit.connections', { defaultValue: '连接' })}</div>
          <strong>{robotCount > 0
            ? t('nomi.cohabit.robotOnline', { count: robotCount, defaultValue: '{{count}} 台机器人已绑定' })
            : t('nomi.cohabit.noRobot', { defaultValue: '尚未绑定机器人' })}</strong>
          <span>{t('nomi.cohabit.channelsHint', { defaultValue: '消息渠道与 MCP 在管理中配置' })}</span>
          <CompanionRobotResourceSettings key={companionId} companionId={companionId} conversation={conversation}
            disabled={quickSettingsSaving || Boolean(conversationError)} onChanged={refreshConversation} onSavingChange={onRobotResourceSavingChange} />
          <button type='button' onClick={() => onManage('remote')}>{t('nomi.cohabit.manageConnections', { defaultValue: '管理渠道与机器人' })}</button>
        </section>

        {status && (
          <section className={styles.infoGroup}>
            <div className={styles.infoTitle}><BookOne size='16' />{t('nomi.cohabit.growth', { defaultValue: '成长' })}</div>
            <div className={styles.growthLine}><strong>Lv {status.level}</strong><span>{status.xp} / {growthNext} XP</span></div>
            <div className={styles.progress}><span style={{ width: `${growthPercent}%` }} /></div>
          </section>
        )}
      </aside>}
    </div>
  );
};

export default CompanionCohabitView;
