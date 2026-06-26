/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Drawer, Select, Spin, Switch } from '@arco-design/web-react';
import { Comment } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { TAssignment, TFleetMember } from '@/common/types/orchestrator/orchestratorTypes';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import TeamChatView from '@/renderer/pages/conversation/components/multiAgent/TeamChatView';
import type { OpenTaskPayload } from './DagCanvas';
import { memberLogo, memberShortLabel } from './memberLabel';

type WorkerTranscriptPanelProps = {
  /** The clicked DAG node's payload (task + assignment + fleet snapshot + refetch).
   * Null = nothing to show (drawer closed). */
  open: OpenTaskPayload | null;
  onClose: () => void;
};

/** One reassign Select option: friendly agent/model label + role hint, plus a logo. */
const memberOption = (m: TFleetMember, roleLabel: (role: string) => string) => {
  const label = memberShortLabel(m) ?? m.id;
  const role = m.role_hint ? roleLabel(m.role_hint) : null;
  return { member: m, label, role, logo: memberLogo(m) };
};

/**
 * Side drawer for one orchestration task. Two stacked sections:
 *
 *  1. Assignment inspector — WHY the task was routed to its member (the
 *     orchestrator's `rationale`), plus controls to **reassign** it to another
 *     fleet member and **lock** the assignment so the auto-router won't override
 *     it. Changes call `PUT …/assignment` and then refetch the run so the canvas
 *     and this panel reflect the new state.
 *  2. Worker transcript — the live, read-only conversation record (mirrors
 *     SubagentDrawer: TeamChatView with the send box hidden). Shown only once a
 *     worker has picked up the task and a conversation exists.
 *
 * `TRunTask.conversation_id` is already the backend INTEGER id, passed straight
 * through with no conversion (unlike TeamAgent.conversation_id, a string).
 */
const WorkerTranscriptPanel: React.FC<WorkerTranscriptPanelProps> = ({ open, onClose }) => {
  const { t } = useTranslation();
  const [message, ctx] = useArcoMessage();
  const [conversation, setConversation] = useState<TChatConversation | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  const task = open?.task ?? null;
  const assignment = open?.assignment ?? null;
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

  const roleLabel = (role: string) =>
    t(`orchestrator.fleet.role.${role}` as 'orchestrator.fleet.role.planner', { defaultValue: role });

  // Apply a reassignment / lock change, then refetch so the canvas + panel sync.
  const applyReassign = async (memberId: string, locked: boolean) => {
    if (!open || !assignment) return;
    setSaving(true);
    try {
      await ipcBridge.orchestrator.runs.reassign.invoke({
        run_id: open.runId,
        task_id: open.task.id,
        updates: { member_id: memberId, locked },
      });
      message.success(t('orchestrator.run.assign.reassignSuccess'));
      await open.refetch();
    } catch (e) {
      message.error(t('orchestrator.run.assign.reassignError', { error: String(e) }));
    } finally {
      setSaving(false);
    }
  };

  const fleetMembers = open?.fleetMembers ?? [];
  const currentMember = assignment ? fleetMembers.find((m) => m.id === assignment.member_id) : undefined;

  return (
    <Drawer
      width={560}
      visible={!!task}
      onCancel={onClose}
      footer={null}
      title={<span className='min-w-0 truncate pr-8px'>{task?.title}</span>}
    >
      {ctx}
      <div className='flex flex-col h-full overflow-hidden'>
        {/* ── Assignment inspector ─────────────────────────────────────────── */}
        {assignment && (
          <div
            className='mb-12px shrink-0 rd-12px p-12px'
            style={{ background: 'var(--bg-2)', border: '1px solid var(--border-base)' }}
          >
            {/* Rationale: "why this member" */}
            <div className='text-11px font-600 uppercase tracking-wide text-t-tertiary'>
              {t('orchestrator.run.assign.rationaleTitle')}
            </div>
            <div className='mt-4px text-13px leading-19px text-t-primary'>
              {assignment.rationale?.trim() || t('orchestrator.run.assign.noRationale')}
            </div>
            {currentMember && (
              <div className='mt-6px flex items-center gap-6px text-12px text-t-secondary'>
                {memberLogo(currentMember) ? (
                  <img src={memberLogo(currentMember) ?? ''} alt='' className='size-14px shrink-0 object-contain' />
                ) : null}
                <span className='truncate'>{memberShortLabel(currentMember) ?? currentMember.id}</span>
                {typeof assignment.score === 'number' && (
                  <span className='shrink-0 text-t-tertiary'>
                    {t('orchestrator.run.assign.score', { score: assignment.score.toFixed(2) })}
                  </span>
                )}
              </div>
            )}

            {/* Reassign + lock controls */}
            <div className='mt-12px flex flex-col gap-8px'>
              <div>
                <div className='mb-4px text-11px font-500 text-t-tertiary'>
                  {t('orchestrator.run.assign.reassign')}
                </div>
                <Select
                  className='w-full'
                  size='small'
                  disabled={saving}
                  value={assignment.member_id}
                  onChange={(memberId: string) => void applyReassign(memberId, assignment.locked)}
                  showSearch
                  filterOption={(input, option) => {
                    const id = (option as React.ReactElement<{ value?: string }>)?.props?.value;
                    const m = fleetMembers.find((fm) => fm.id === id);
                    const text = `${m?.agent_id ?? ''} ${m?.model ?? ''} ${m?.role_hint ?? ''}`.toLowerCase();
                    return text.includes(input.toLowerCase());
                  }}
                >
                  {fleetMembers.map((m) => {
                    const opt = memberOption(m, roleLabel);
                    return (
                      <Select.Option key={m.id} value={m.id}>
                        <span className='flex items-center gap-8px'>
                          <span className='size-16px shrink-0 flex items-center justify-center'>
                            {opt.logo ? (
                              <img src={opt.logo} alt='' className='size-16px object-contain' />
                            ) : (
                              <span className='text-12px leading-none'>🤖</span>
                            )}
                          </span>
                          <span className='truncate'>{opt.label}</span>
                          {opt.role && <span className='shrink-0 text-t-tertiary text-11px'>· {opt.role}</span>}
                        </span>
                      </Select.Option>
                    );
                  })}
                </Select>
              </div>
              <div className='flex items-center justify-between'>
                <div className='flex flex-col'>
                  <span className='text-12px font-500 text-t-primary'>{t('orchestrator.run.assign.locked')}</span>
                  <span className='text-11px leading-15px text-t-tertiary'>{t('orchestrator.run.assign.lockedHint')}</span>
                </div>
                <Switch
                  size='small'
                  disabled={saving}
                  checked={assignment.locked}
                  onChange={(locked: boolean) => void applyReassign(assignment.member_id, locked)}
                />
              </div>
            </div>
          </div>
        )}

        {/* ── Worker transcript ─────────────────────────────────────────────── */}
        <div className='flex flex-1 min-h-0 flex-col overflow-hidden'>
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
      </div>
    </Drawer>
  );
};

export default WorkerTranscriptPanel;
