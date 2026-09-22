/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import useSWR from 'swr';
import { Button, Spin } from '@arco-design/web-react';
import { BookOne, Connection, Data, Right, RobotOne, SettingTwo } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { CompanionId, ConversationId } from '@/common/types/ids';
import { getConversationOrNull } from '@/renderer/pages/conversation/utils/conversationCache';
import { ExecutionProvider } from '@/renderer/pages/conversation/execution/ExecutionContext';
import { PreviewProvider } from '@/renderer/pages/conversation/Preview';
import { browserStorageKey } from '@/common/utils/browserStorageKey';
import CompanionConversation from '../companion/CompanionConversation';
import type { CompanionHandle, WorkspaceTabKey } from './types';
import styles from './CompanionCohabitView.module.css';

type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

interface Props {
  companionId: CompanionId;
  companion: CompanionHandle;
  onManage: (tab: WorkspaceTabKey) => void;
}

const CompanionCohabitView: React.FC<Props> = ({ companionId, companion, onManage }) => {
  const { t } = useTranslation();
  const { profile, status } = companion;
  const [conversationId, setConversationId] = useState<ConversationId | null>(null);
  const [sessionError, setSessionError] = useState(false);
  const [robotCount, setRobotCount] = useState(0);

  useEffect(() => {
    setConversationId(null);
    setSessionError(false);
    if (!profile?.model) return;
    let cancelled = false;
    void ipcBridge.companion.ensureCompanionSession
      .invoke({ companion_id: companionId })
      .then((session) => {
        if (!cancelled) setConversationId(session.conversation_id);
      })
      .catch(() => {
        if (!cancelled) setSessionError(true);
      });
    return () => {
      cancelled = true;
    };
  }, [companionId, profile?.model?.provider_id, profile?.model?.model]);

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

  const { data: conversation, isLoading, mutate } = useSWR<NomiConversation | null>(
    conversationId ? `conversation/${conversationId}` : null,
    async () => {
      const row = await getConversationOrNull(conversationId!);
      return row?.type === 'nomi' ? row : null;
    }
  );

  useEffect(() => {
    if (!conversationId) return;
    return ipcBridge.conversation.listChanged.on((event) => {
      if (event.conversation_id === conversationId && event.action !== 'deleted') void mutate();
    });
  }, [conversationId, mutate]);

  const memoryCount = (status?.memories_active ?? 0) + (status?.memories_archived ?? 0);
  const modelLabel = profile?.model?.model ?? t('nomi.overview.modelMissingShort', { defaultValue: '未配置' });
  const growthBase = status ? Math.max(0, (status.level - 1) ** 2 * 100) : 0;
  const growthNext = status ? Math.max(100, status.level ** 2 * 100) : 100;
  const growthPercent = status
    ? Math.max(0, Math.min(100, Math.round(((status.xp - growthBase) / (growthNext - growthBase)) * 100)))
    : 0;

  const chat = useMemo(() => {
    if (!profile?.model) {
      return (
        <div className={styles.chatState}>
          <RobotOne theme='outline' size='30' fill='currentColor' />
          <strong>{t('nomi.cohabit.modelRequired', { defaultValue: '先为伙伴配置主对话模型' })}</strong>
          <span>{t('nomi.cohabit.modelRequiredHint', { defaultValue: '配置后即可在这里延续桌面、IM 与机器人的同一段相处历史。' })}</span>
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
        <ExecutionProvider conversation={conversation}>
          <CompanionConversation conversation={conversation} companion={companion} compact />
        </ExecutionProvider>
      </PreviewProvider>
    );
  }, [companion, conversation, conversationId, isLoading, onManage, profile?.model, sessionError, t]);

  return (
    <div className={styles.layout}>
      <section className={styles.chatColumn}>
        <div className={styles.chatHeading}>
          <h1>{t('nomi.cohabit.title', { name: profile?.name ?? '', defaultValue: '和{{name}}相处' })}</h1>
          <p>{t('nomi.cohabit.subtitle', { defaultValue: '分享日常、聊聊想法，让伙伴真正陪在身边。' })}</p>
        </div>
        <div className={styles.chatBody}>{chat}</div>
      </section>

      <aside className={styles.info} aria-label={t('nomi.cohabit.infoTitle', { defaultValue: '伙伴信息' })}>
        <div className={styles.infoHeader}>
          <h2>{t('nomi.cohabit.infoTitle', { defaultValue: '伙伴信息' })}</h2>
          <button type='button' onClick={() => onManage('overview')}>
            {t('nomi.workspace.manage', { defaultValue: '管理' })}<Right size='13' />
          </button>
        </div>

        <section className={styles.infoGroup}>
          <div className={styles.infoTitle}><SettingTwo size='16' />{t('nomi.cohabit.currentState', { defaultValue: '当前状态' })}</div>
          <strong>{t(`nomi.moods.${status?.mood ?? 'content'}`, { defaultValue: status?.mood ?? 'content' })}</strong>
          <span>{profile?.appearance.companion_enabled
            ? t('nomi.cohabit.desktopVisible', { defaultValue: '桌面显示已开启' })
            : t('nomi.cohabit.desktopHidden', { defaultValue: '桌面显示已隐藏' })}</span>
          <button type='button' onClick={() => onManage('overview')}>{t('nomi.cohabit.manageState', { defaultValue: '管理形象与状态' })}</button>
        </section>

        <section className={styles.infoGroup}>
          <div className={styles.infoTitle}><RobotOne size='16' />{t('nomi.cohabit.mind', { defaultValue: '心智配置' })}</div>
          <strong>{t('nomi.cohabit.agent', { defaultValue: '伙伴 Agent' })}</strong>
          <span>{modelLabel}</span>
          <button type='button' onClick={() => onManage('overview')}>{t('nomi.cohabit.manageMind', { defaultValue: '管理人格与模型' })}</button>
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
          <button type='button' onClick={() => onManage('remote')}>{t('nomi.cohabit.manageConnections', { defaultValue: '管理连接与设备' })}</button>
        </section>

        {status && (
          <section className={styles.infoGroup}>
            <div className={styles.infoTitle}><BookOne size='16' />{t('nomi.cohabit.growth', { defaultValue: '成长' })}</div>
            <div className={styles.growthLine}><strong>Lv {status.level}</strong><span>{status.xp} / {growthNext} XP</span></div>
            <div className={styles.progress}><span style={{ width: `${growthPercent}%` }} /></div>
          </section>
        )}
      </aside>
    </div>
  );
};

export default CompanionCohabitView;
