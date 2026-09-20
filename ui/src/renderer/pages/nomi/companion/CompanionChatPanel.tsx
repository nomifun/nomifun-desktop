/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Spin } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { refreshConversationCache } from '@/renderer/pages/conversation/utils/conversationCache';
import type { TChatConversation } from '@/common/config/storage';
import ChatSlider from '@/renderer/pages/conversation/components/ChatSlider';
import ExecutionConversationLayout from '@/renderer/pages/conversation/execution/ExecutionConversationLayout';
import { useCompanion } from '../useNomi';
import CompanionConversation from './CompanionConversation';
import CompanionDevicesControl from './CompanionDevicesControl';
import KnowledgeControl from '@/renderer/pages/conversation/components/KnowledgeControl';
import SystemPermissionReminder from '@/renderer/pages/conversation/components/SystemPermissionReminder';
import type { WorkspaceExtraTab } from '@/renderer/pages/conversation/Workspace/types';

type NomiConversation = Extract<TChatConversation, { type: 'nomi' }>;

interface Props {
  /** A desktop-companion's single per-companion nomi session (extra.companion_session). */
  conversation: NomiConversation;
  /** Conversation-owned resources exposed in the shared right-hand tool rail. */
  extraTabs?: WorkspaceExtraTab[];
}

/** Adapts companion resources to the shared conversation shell. */
const CompanionChatPanel: React.FC<Props> = ({ conversation, extraTabs }) => {
  const { t } = useTranslation();
  const companionId = conversation.extra?.companion_id ?? null;
  const companion = useCompanion(companionId);
  const { profile } = companion;
  const workspace = conversation.extra?.workspace ?? '';
  useEffect(() => {
    if (!companionId || conversation.status === 'running') return;
    let cancelled = false;
    // Resolve the current product recipe without warming a model or making
    // another conversation. Official seeds follow updates; custom Agents stay.
    void ipcBridge.companion.ensureCompanionSession.invoke({ companion_id: companionId })
      .then(() => { if (!cancelled) return refreshConversationCache(conversation.id); })
      .catch(() => { if (!cancelled) Message.error(t('agentSettings.productBinding.loadFailed')); });
    return () => { cancelled = true; };
  }, [companionId, conversation.id, conversation.status, t]);

  const renderInExecutionShell = (content: React.ReactNode, showDeviceControl = false) => (
    <ExecutionConversationLayout
      title={conversation.name}
      conversation_id={conversation.id}
      backend='nomi'
      agent_name={profile?.name}
      headerControls={companionId && conversation.agent_snapshot?.required_resource_kinds.includes('knowledge_base')
        ? <KnowledgeControl target={{ kind: 'companion', id: companionId }} /> : null}
      disableRename
      workspaceEnabled={Boolean(workspace)}
      workspacePath={workspace || undefined}
      sider={<ChatSlider conversation={conversation} extraTabs={extraTabs} />}
      siderTitle={<span className='text-16px font-bold text-t-primary'>{t('conversation.workspace.title')}</span>}
      workspaceExtraTabs={extraTabs}
      headerExtra={<div className='flex items-center gap-8px'>
        <SystemPermissionReminder conversationId={conversation.id} snapshot={conversation.agent_snapshot} />
        {showDeviceControl && companionId && <CompanionDevicesControl companion={companion} conversationId={conversation.id} />}
      </div>}
    >
      {content}
    </ExecutionConversationLayout>
  );

  // 会话被标记为伙伴会话但缺 companionId（异常数据）：兜底，避免空白面板。
  if (!companionId) {
    return renderInExecutionShell(
      <div className='flex-1 flex items-center justify-center text-13px text-t-tertiary px-16px text-center'>
        {t('nomi.companion.chatError')}
      </div>,
    );
  }

  // 解析伙伴 profile 中（切伙伴时 useCompanion 同步置空，避免 stale）。
  if (!profile) {
    return renderInExecutionShell(
      <div className='flex-1 flex justify-center items-center py-40px'>
        <Spin />
      </div>,
    );
  }

  return renderInExecutionShell(
    <CompanionConversation conversation={conversation} companion={companion} />,
    true,
  );
};

export default CompanionChatPanel;
