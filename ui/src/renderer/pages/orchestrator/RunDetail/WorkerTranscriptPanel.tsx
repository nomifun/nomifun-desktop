/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Drawer, Spin } from '@arco-design/web-react';
import { Comment } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { TRunTask } from '@/common/types/orchestrator/orchestratorTypes';
import TeamChatView from '@/renderer/pages/conversation/components/multiAgent/TeamChatView';

type WorkerTranscriptPanelProps = {
  /** The clicked DAG node's task. Null = nothing to show (drawer closed). */
  task: TRunTask | null;
  onClose: () => void;
};

/**
 * Read-only side drawer that mirrors one orchestration worker's live conversation
 * transcript (spec §6 / Task 5). Clicking a DAG node opens this panel for that task.
 *
 * Mirrors SubagentDrawer: slides in from the right, body embeds TeamChatView with
 * the send box hidden — a progress viewer, not an input surface. The worker's full
 * conversation record is fetched by id when the drawer opens; NomiChat self-mounts
 * its providers and merges the live streaming, so the transcript updates in real
 * time without any send box.
 *
 * Unlike TeamAgent.conversation_id (a string), TRunTask.conversation_id is already
 * the backend INTEGER id — it is passed straight through with no conversion. When a
 * task has no conversation yet (not picked up by a worker), we show a themed empty
 * state instead of attempting to load.
 */
const WorkerTranscriptPanel: React.FC<WorkerTranscriptPanelProps> = ({ task, onClose }) => {
  const { t } = useTranslation();
  const [conversation, setConversation] = useState<TChatConversation | null>(null);
  const [loading, setLoading] = useState(false);

  const conversationId = task?.conversation_id;

  useEffect(() => {
    if (!task || conversationId === undefined) {
      setConversation(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    // `TRunTask.conversation_id` is already the backend INTEGER id — no conversion.
    void ipcBridge.conversation.get
      .invoke({ id: conversationId })
      .then((conv) => {
        if (!cancelled) setConversation((conv as TChatConversation | null) ?? null);
      })
      .catch((e) => {
        console.error('[WorkerTranscriptPanel] load conversation failed:', e);
        if (!cancelled) setConversation(null);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [task, conversationId]);

  return (
    <Drawer
      width={560}
      visible={!!task}
      onCancel={onClose}
      footer={null}
      title={<span className='min-w-0 truncate pr-8px'>{task?.title}</span>}
    >
      <div className='flex flex-col h-full overflow-hidden'>
        {conversationId === undefined ? (
          <div className='flex size-full flex-col items-center justify-center gap-12px px-24px text-center'>
            <span className='flex size-52px items-center justify-center rd-16px bg-fill-2 text-t-tertiary'>
              <Comment theme='outline' size='26' strokeWidth={3} />
            </span>
            <div className='text-15px font-600 text-t-primary'>{t('orchestrator.run.transcript.notStarted')}</div>
            <div className='max-w-320px text-12px leading-18px text-t-tertiary'>
              {t('orchestrator.run.transcript.noConversation')}
            </div>
          </div>
        ) : loading ? (
          <Spin loading className='flex flex-1 items-center justify-center' />
        ) : conversation ? (
          <TeamChatView conversation={conversation} hideSendBox agent_name={task?.title} />
        ) : null}
      </div>
    </Drawer>
  );
};

export default WorkerTranscriptPanel;
