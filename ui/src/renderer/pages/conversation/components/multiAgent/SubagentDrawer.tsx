/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Drawer, Spin } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { TeamAgent } from '@/common/types/team/teamTypes';
import AgentStatusBadge from './AgentStatusBadge';
import TeamChatView from './TeamChatView';

type SubagentDrawerProps = {
  visible: boolean;
  /** The selected subagent whose work progress is previewed. Null = nothing to show. */
  agent: TeamAgent | null;
  onClose: () => void;
};

/**
 * Read-only side drawer that previews one subagent's conversation (spec §6).
 * Mirrors the RequirementDetailDrawer pattern: slides in from the right with a
 * title (agent name + status badge) and a close affordance. The body embeds
 * TeamChatView with the send box hidden — this is a progress viewer, not an
 * input surface. The subagent's full conversation record is fetched by id when
 * the drawer opens (the status strip only carries lightweight agent metadata).
 */
const SubagentDrawer: React.FC<SubagentDrawerProps> = ({ visible, agent, onClose }) => {
  const { t } = useTranslation();
  const [conversation, setConversation] = useState<TChatConversation | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!visible || !agent?.conversation_id) {
      setConversation(null);
      return;
    }
    let cancelled = false;
    setLoading(true);
    // `TeamAgent.conversation_id` is a string in the team type; the conversation
    // API takes the backend INTEGER id (numeric-id spec §2) — convert at the boundary.
    void ipcBridge.conversation.get
      .invoke({ id: Number(agent.conversation_id) })
      .then((conv) => {
        if (!cancelled) setConversation((conv as TChatConversation | null) ?? null);
      })
      .catch((e) => {
        console.error('[SubagentDrawer] load conversation failed:', e);
        if (!cancelled) setConversation(null);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [visible, agent?.conversation_id]);

  return (
    <Drawer
      width={520}
      visible={visible}
      onCancel={onClose}
      footer={null}
      title={
        <div className='flex items-center gap-8px min-w-0 pr-8px'>
          {agent ? <AgentStatusBadge status={agent.status} /> : null}
          <span className='min-w-0 truncate'>
            {agent ? t('multiAgent.drawer.title', { name: agent.agent_name }) : null}
          </span>
          {agent ? (
            <span className='shrink-0 text-t-tertiary text-12px font-400'>
              {t(`multiAgent.status.${agent.status}` as const)}
            </span>
          ) : null}
        </div>
      }
    >
      <div className='flex flex-col h-full overflow-hidden'>
        {loading ? (
          <Spin loading className='flex flex-1 items-center justify-center' />
        ) : conversation ? (
          <TeamChatView conversation={conversation} hideSendBox agent_name={agent?.agent_name} agent_icon={agent?.icon} />
        ) : null}
      </div>
    </Drawer>
  );
};

export default SubagentDrawer;
