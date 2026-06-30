/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import type { TChatConversation } from '@/common/config/storage';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import React from 'react';
import { useTranslation } from 'react-i18next';
import ChatWorkspace from '../Workspace';
import NomiSessionMetricsPanel from '../platforms/nomi/NomiSessionMetricsPanel';
import OrchestrationRailTab from '../orchestration/OrchestrationRailTab';
import { useOrchestrationSafe } from '../orchestration/OrchestrationContext';

const ChatSlider: React.FC<{
  conversation?: TChatConversation;
  /**
   * One-shot initial active tab for the nomi rail (today only `'orchestration'`,
   * set by the orchestration landing flow). The flag is read + cleared ONCE in
   * the owner ({@link NomiConversationPanel}); ChatSlider just forwards it. Other
   * surfaces leave it unset → the rail defaults to the Files tab.
   */
  defaultRailTab?: 'orchestration';
}> = ({ conversation, defaultRailTab }) => {
  const [messageApi, messageContext] = useArcoMessage({ maxCount: 1 });
  const { t } = useTranslation();
  // F5 carry-forward fix: the 「编排」tab is only meaningful when an
  // OrchestrationProvider is in scope (the main conversation surface). The
  // companion 聊天 tab renders ChatSlider WITHOUT a provider → `orch == null` →
  // the tab is omitted there. Hook is called unconditionally (Rules of Hooks).
  const orch = useOrchestrationSafe();

  // Only select the 编排 tab on landing when the orchestration provider is in
  // scope (a real run surface). The owner already read + cleared the flag once.
  const defaultOrchestrationTab = defaultRailTab === 'orchestration' && orch != null;

  let workspaceNode: React.ReactNode = null;
  if (conversation?.type === 'acp' && conversation.extra?.workspace) {
    workspaceNode = (
      <ChatWorkspace
        conversation_id={conversation.id}
        workspace={conversation.extra.workspace}
        isTemporaryWorkspace={
          (conversation.extra as { is_temporary_workspace?: boolean } | undefined)?.is_temporary_workspace
        }
        eventPrefix='acp'
        messageApi={messageApi}
      ></ChatWorkspace>
    );
  } else if (conversation?.type === 'codex' && conversation.extra?.workspace) {
    workspaceNode = (
      <ChatWorkspace
        conversation_id={conversation.id}
        workspace={conversation.extra.workspace}
        isTemporaryWorkspace={
          (conversation.extra as { is_temporary_workspace?: boolean } | undefined)?.is_temporary_workspace
        }
        eventPrefix='codex'
        messageApi={messageApi}
      ></ChatWorkspace>
    );
  } else if (conversation?.type === 'nomi' && conversation.extra?.workspace) {
    workspaceNode = (
      <ChatWorkspace
        conversation_id={conversation.id}
        workspace={conversation.extra.workspace}
        isTemporaryWorkspace={
          (conversation.extra as { is_temporary_workspace?: boolean } | undefined)?.is_temporary_workspace
        }
        eventPrefix='nomi'
        messageApi={messageApi}
        defaultActiveTab={defaultOrchestrationTab ? 'orchestration' : undefined}
        extraTabs={[
          ...(orch != null
            ? [
                {
                  key: 'orchestration',
                  title: t('conversation.orchestration.tab', { defaultValue: '编排' }),
                  content: <OrchestrationRailTab />,
                },
              ]
            : []),
          {
            key: 'nomi-session-metrics',
            title: t('conversation.sessionMetrics.tab'),
            content: <NomiSessionMetricsPanel conversation={conversation} />,
          },
        ]}
      ></ChatWorkspace>
    );
  } else if (conversation?.type === 'openclaw-gateway' && conversation.extra?.workspace) {
    workspaceNode = (
      <ChatWorkspace
        conversation_id={conversation.id}
        workspace={conversation.extra.workspace}
        isTemporaryWorkspace={
          (conversation.extra as { is_temporary_workspace?: boolean } | undefined)?.is_temporary_workspace
        }
        eventPrefix='openclaw-gateway'
        messageApi={messageApi}
      ></ChatWorkspace>
    );
  } else if (conversation?.type === 'nanobot' && conversation.extra?.workspace) {
    workspaceNode = (
      <ChatWorkspace
        conversation_id={conversation.id}
        workspace={conversation.extra.workspace}
        isTemporaryWorkspace={
          (conversation.extra as { is_temporary_workspace?: boolean } | undefined)?.is_temporary_workspace
        }
        eventPrefix='nanobot'
        messageApi={messageApi}
      ></ChatWorkspace>
    );
  } else if (conversation?.type === 'remote' && conversation.extra?.workspace) {
    workspaceNode = (
      <ChatWorkspace
        conversation_id={conversation.id}
        workspace={conversation.extra.workspace}
        isTemporaryWorkspace={
          (conversation.extra as { is_temporary_workspace?: boolean } | undefined)?.is_temporary_workspace
        }
        eventPrefix='remote'
        messageApi={messageApi}
      ></ChatWorkspace>
    );
  }

  if (!workspaceNode) {
    return <div></div>;
  }

  return (
    <>
      {messageContext}
      {workspaceNode}
    </>
  );
};

export default ChatSlider;
