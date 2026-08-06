/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { TChatConversation } from '@/common/config/storage';
import type { ConversationId, SshHostId } from '@/common/types/ids';
import { Tooltip } from '@arco-design/web-react';
import { Server } from '@icon-park/react';
import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import useSWR from 'swr';

import { useConversationListSync } from './hooks/useConversationListSync';

interface Props {
  /** Active conversation id parsed from the `/conversation/:id` route. */
  activeConversationId: ConversationId | null;
  /** Icon-only rail variant (parent sider collapsed). */
  collapsed?: boolean;
  /** Fold state of the group (persisted in useWorkpathUiState). Ignored in the collapsed rail. */
  expanded?: boolean;
  /** Toggles the persisted fold state. */
  onToggleExpanded?: () => void;
  /**
   * Row renderer supplied by the container (a fully wired conversation row).
   * Same seam as WorkpathDrawer's `renderEntry`: the row's action props live in
   * SessionList/index.tsx and are not reachable from here.
   */
  renderRow: (conversation: TChatConversation) => React.ReactNode;
}

/**
 * 会话侧边栏顶部的「SSH 会话」专属分组（设计 §10 的顶层独立分组）。
 *
 * 绑定了远程主机的会话（`extra.ssh_host_id`）被 `isOrdinaryWorkConversation`
 * 排除出普通工作会话列表 —— 那个排除是对的（远程会话有自己的归属），但在本组
 * 落地之前它等于「建完一切走就再也找不回来」。本组补上缺的另一半：
 *
 * - 数据来自 `useConversationListSync` 快照里的 `sshConversations`，与普通列表
 *   共用同一次列表拉取，本组自己不发任何请求；
 * - 二级按主机聚合：主机名作子标题，其会话列在下面（主机名取自主机簿，与
 *   设置页共用同一个 SWR key，SWR 自动去重）；
 * - 主机被删掉而会话还在时，用诚实的兜底标题继续展示，绝不静默丢行。
 *
 * 不触碰过滤器本身，也不自建行：行由容器通过 `renderRow` 注入（同
 * WorkpathDrawer 的 `renderEntry` 约定）。
 */
const SshSessionGroup: React.FC<Props> = ({
  activeConversationId,
  collapsed = false,
  expanded = true,
  onToggleExpanded,
  renderRow,
}) => {
  const { t } = useTranslation();
  const { sshConversations } = useConversationListSync();
  // Same SWR key as the host book (SshHostSettings/SshHostManagement), so both
  // mounts share one `/api/ssh-hosts` request.
  const { data: hosts } = useSWR('ssh-hosts.list', () => ipcBridge.ssh.list.invoke());

  const hostNames = useMemo(() => {
    const names = new Map<SshHostId, string>();
    for (const host of hosts ?? []) names.set(host.sshHostId, host.name);
    return names;
  }, [hosts]);

  // Second level = host. Buckets keep the snapshot's order (recent first) and
  // first-seen host order, so the group is stable across refreshes.
  const hostGroups = useMemo(() => {
    const byHost = new Map<SshHostId, TChatConversation[]>();
    for (const conversation of sshConversations) {
      const sshHostId = (conversation.extra as { ssh_host_id?: SshHostId } | undefined)?.ssh_host_id;
      if (sshHostId == null) continue;
      const bucket = byHost.get(sshHostId);
      if (bucket) bucket.push(conversation);
      else byHost.set(sshHostId, [conversation]);
    }
    return Array.from(byHost.entries());
  }, [sshConversations]);

  /**
   * The host book answers asynchronously and hosts can be deleted while their
   * sessions survive, so an unresolved id gets a label, never a dropped row:
   * - book not answered yet (loading or failed) → 未知主机, no accusation;
   * - book answered and the id is absent → the host really is gone.
   */
  const hostLabelOf = (sshHostId: SshHostId): string =>
    hostNames.get(sshHostId) ?? (hosts == null ? t('ssh.group.hostUnknown') : t('ssh.group.hostMissing'));

  // No host-bound session → no group at all (mirrors CompanionSessionGroup).
  if (hostGroups.length === 0) return null;

  if (collapsed) {
    // Icon rail: the rows render themselves icon-only with a tooltip (the same
    // collapsed treatment the ordinary collapsed list gets). Host sub-headers
    // have no room in a 36px rail, so the sessions are flat here.
    return <div className='min-w-0'>{sshConversations.map((conversation) => renderRow(conversation))}</div>;
  }

  // A collapsed group must never hide the session you are currently in — that is
  // the very bug this group exists to fix. Visual override only, never persisted.
  const holdsActiveSession =
    activeConversationId != null && sshConversations.some((conversation) => conversation.id === activeConversationId);
  const showSessions = expanded || holdsActiveSession;

  return (
    <div className='min-w-0 mb-2px'>
      {/* 与「桌面伙伴」「项目/工作路径」完全同款的纯 section 标题（无边框/箭头，
          只有标签 + 数字）。点击整行切换持久化折叠态（默认展开）。 */}
      <div className='px-2px'>
        <div
          className='h-22px px-2px flex items-center justify-between gap-8px select-none cursor-pointer min-w-0'
          onClick={() => onToggleExpanded?.()}
        >
          <span className='text-13px text-t-tertiary font-[500] leading-none tracking-wide truncate min-w-0'>
            {t('ssh.sessionGroup')}
          </span>
          <span className='text-12px text-t-tertiary leading-none shrink-0'>{sshConversations.length}</span>
        </div>
      </div>

      {showSessions && (
        <div className='flex flex-col gap-2px mt-2px'>
          {hostGroups.map(([sshHostId, conversations]) => {
            const hostLabel = hostLabelOf(sshHostId);
            return (
              <div key={sshHostId} className='min-w-0'>
                {/* 二级主机子标题：与 SessionKindGroup 的 kind 子标题同款排版
                    （h-26px / pl-22px / 12px 三级文字 + 计数）。 */}
                <Tooltip content={t('ssh.group.hostTooltip', { host: hostLabel })} position='top'>
                  <div className='flex items-center gap-4px h-26px pl-22px pr-8px select-none min-w-0'>
                    <span className='size-18px flex items-center justify-center shrink-0 text-t-tertiary'>
                      <Server theme='outline' size={12} fill='currentColor' className='block leading-none' />
                    </span>
                    <span className='text-12px text-t-tertiary font-[500] leading-none min-w-0 truncate'>
                      {hostLabel}
                    </span>
                    <span className='text-12px text-t-tertiary leading-none shrink-0'>({conversations.length})</span>
                  </div>
                </Tooltip>
                <div className='min-w-0 flex flex-col'>
                  {conversations.map((conversation) => renderRow(conversation))}
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};

export default SshSessionGroup;
