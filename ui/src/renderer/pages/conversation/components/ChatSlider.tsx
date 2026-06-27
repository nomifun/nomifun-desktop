/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import type { TChatConversation } from '@/common/config/storage';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import React, { Suspense } from 'react';
import { useTranslation } from 'react-i18next';
import ChatWorkspace from '../Workspace';
import NomiSessionMetricsPanel from '../platforms/nomi/NomiSessionMetricsPanel';

// The orchestration DAG tab pulls in react-flow (heavy) and is only rendered
// for a lead conversation with a run, so it is split into its own chunk.
const DagRailTab = React.lazy(() => import('@/renderer/pages/orchestrator/RunDetail/DagRailTab'));

const ChatSlider: React.FC<{
  conversation?: TChatConversation;
}> = ({ conversation }) => {
  const [messageApi, messageContext] = useArcoMessage({ maxCount: 1 });
  const { t } = useTranslation();

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
    // A "lead" orchestrator conversation carries its run id on `extra`
    // (untyped — read via a narrow cast). When present, append a 「编排」rail tab
    // that embeds the run's DAG canvas + worker transcript drawer.
    const orchestratorExtra = conversation.extra as
      | { orchestrator_role?: string; orchestrator_run_id?: string }
      | undefined;
    const leadRunId =
      orchestratorExtra?.orchestrator_role === 'lead' && orchestratorExtra.orchestrator_run_id
        ? orchestratorExtra.orchestrator_run_id
        : undefined;
    workspaceNode = (
      <ChatWorkspace
        conversation_id={conversation.id}
        workspace={conversation.extra.workspace}
        isTemporaryWorkspace={
          (conversation.extra as { is_temporary_workspace?: boolean } | undefined)?.is_temporary_workspace
        }
        eventPrefix='nomi'
        messageApi={messageApi}
        extraTabs={[
          {
            key: 'nomi-session-metrics',
            title: t('conversation.sessionMetrics.tab'),
            content: <NomiSessionMetricsPanel conversation={conversation} />,
          },
          ...(leadRunId
            ? [
                {
                  key: 'orchestrator-dag',
                  title: t('orchestrator.run.dagTab'),
                  content: (
                    <Suspense fallback={null}>
                      <DagRailTab runId={leadRunId} />
                    </Suspense>
                  ),
                },
              ]
            : []),
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
