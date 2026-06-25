/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { TeamAgent, TTeam } from '@/common/types/team/teamTypes';
import { normalizeMultiAgentConfig } from './multiAgentConfig';
import { useTeamSession } from './hooks/useTeamSession';
import AgentStatusBadge from './AgentStatusBadge';
import SubagentDrawer from './SubagentDrawer';

type AgentStatusStripProps = {
  /** The conversation acting as the multi-agent leader (spec §6).
   *  Backend INTEGER id (numeric-id spec §1). */
  conversation_id: number;
};

/** Field name on `extra` is snake_case `team_id`; tolerate a legacy `teamId`
 *  (mirrors TeamChatEmptyState / useConversationListSync). */
function readTeamId(conversation: TChatConversation | null | undefined): string | undefined {
  const extra = conversation?.extra as { team_id?: string; teamId?: string } | undefined;
  return extra?.team_id ?? extra?.teamId;
}

/**
 * Inner body: rendered only once the team record is loaded so `useTeamSession`
 * (which needs a concrete TTeam) is never called with a stale placeholder.
 * Shows one status chip per non-leader subagent; clicking a chip opens the
 * read-only preview drawer. The leader is the current conversation itself, so
 * it is intentionally not repeated as a chip.
 */
const StripBody: React.FC<{ team: TTeam }> = ({ team }) => {
  const { t } = useTranslation();
  const { statusMap } = useTeamSession(team);
  const [selected, setSelected] = useState<TeamAgent | null>(null);

  const subagents = team.agents.filter((a) => a.role !== 'leader');

  if (subagents.length === 0) {
    // Auto mode before the leader has spawned anyone: keep a quiet placeholder
    // rather than a zero-height strip, so the affordance is discoverable.
    return (
      <div className='px-12px py-6px text-t-quaternary text-12px leading-16px shrink-0'>
        {t('multiAgent.subagents.empty')}
      </div>
    );
  }

  return (
    <>
      <div className='flex items-center gap-8px px-12px py-6px overflow-x-auto scrollbar-hide shrink-0'>
        <span className='text-t-quaternary text-12px shrink-0'>{t('multiAgent.subagents.title')}</span>
        {subagents.map((agent) => {
          const status = statusMap.get(agent.slot_id)?.status ?? agent.status;
          return (
            <button
              key={agent.slot_id}
              type='button'
              title={t('multiAgent.subagents.openPreview')}
              onClick={() => setSelected(agent)}
              className='inline-flex items-center gap-6px shrink-0 rounded-full border border-solid border-border-2 bg-fill-1 px-10px py-3px text-12px text-t-secondary leading-none hover:bg-fill-2 transition-colors'
            >
              <AgentStatusBadge status={status} />
              <span className='max-w-120px truncate'>{agent.agent_name}</span>
            </button>
          );
        })}
      </div>
      <SubagentDrawer visible={!!selected} agent={selected} onClose={() => setSelected(null)} />
    </>
  );
};

/**
 * Subagent status strip shown above the input box when the conversation has
 * multi-agent collaboration enabled (spec §6). Self-gates: it reads the
 * conversation's `extra` to confirm `multi_agent.enabled` and resolve the
 * `team_id`, then fetches the team record. The live per-agent status comes from
 * `useTeamSession` (team WS events). It renders nothing until both the gate and
 * the team resolve, so the layout never flickers a frame of stale chips.
 */
const AgentStatusStrip: React.FC<AgentStatusStripProps> = ({ conversation_id }) => {
  const { data: conversation } = useSWR(conversation_id ? `conversation/${conversation_id}` : null, () =>
    ipcBridge.conversation.get.invoke({ id: conversation_id })
  );

  const multiAgent = normalizeMultiAgentConfig((conversation?.extra as { multi_agent?: unknown } | undefined)?.multi_agent);
  const teamId = readTeamId(conversation);
  const active = multiAgent.enabled && !!teamId;

  const { data: team } = useSWR(active && teamId ? `team/${teamId}` : null, () =>
    ipcBridge.team.get.invoke({ id: teamId as string })
  );

  if (!active || !team) return null;
  return <StripBody team={team} />;
};

export default AgentStatusStrip;
