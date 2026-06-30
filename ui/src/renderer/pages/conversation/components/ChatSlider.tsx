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
}> = ({ conversation }) => {
  const [messageApi, messageContext] = useArcoMessage({ maxCount: 1 });
  const { t } = useTranslation();
  // F5 carry-forward fix: the 「编排」tab is only meaningful when an
  // OrchestrationProvider is in scope (the main conversation surface). The
  // companion 聊天 tab renders ChatSlider WITHOUT a provider → `orch == null` →
  // the tab is omitted there. Hook is called unconditionally (Rules of Hooks).
  const orch = useOrchestrationSafe();

  // Landing display: when this conversation was launched from the homepage
  // 「智能编排」entry (useGuidSend stashed `nomi_open_rail_<id>`) AND it actually
  // has a linked run, open the right rail straight onto the 编排 tab — NO
  // floating window. The rail is opened via the persisted workspace preference
  // (written alongside the flag); here we only decide the initial active tab and
  // consume the flag once so it never re-selects after the user switches tabs.
  // `useMemo` over the run-presence so the one-shot read is stable across the
  // re-renders that follow `runId` lighting up.
  const orchHasRun = orch?.runId != null;
  const defaultOrchestrationTab = React.useMemo(() => {
    if (!orchHasRun || conversation == null) return false;
    const key = `nomi_open_rail_${conversation.id}`;
    try {
      if (sessionStorage.getItem(key) == null) return false;
      sessionStorage.removeItem(key);
      return true;
    } catch {
      return false;
    }
  }, [orchHasRun, conversation]);

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
