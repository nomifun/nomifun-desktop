/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Spin } from '@arco-design/web-react';
import React, { useEffect, useRef } from 'react';

import RemoteConnectSection from './RemoteConnectSection';
import type { WorkspaceTabProps } from '../../types';
import AccessTokenSection from './AccessTokenSection';
import { usePairingAttention } from './usePairingAttention';

/**
 * 远程控制 tab：从桌面应用之外触达这只伙伴的两条路径 —— IM 渠道（谁来接待）
 * 与远程访问令牌（外部客户端以它的身份接入）。
 *
 * Remote control tab. Two sections, one idea each: the IM channel bots this
 * companion greets on, and the per-companion access token for external
 * MCP / REST clients. Attention (the strip dot) means a pairing request on one
 * of this companion's bots is waiting for approval.
 */
const RemoteTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { profile, status } = companion;

  const pendingPairings = usePairingAttention(profile ? companionId : null);

  const attentionRef = useRef(onAttentionChange);
  attentionRef.current = onAttentionChange;
  useEffect(() => {
    attentionRef.current?.(pendingPairings > 0);
  }, [pendingPairings]);
  // Leaving the tab must not leave a stale dot behind.
  useEffect(() => () => attentionRef.current?.(false), []);

  if (!profile) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  return (
    <div className='flex flex-col gap-16px py-8px'>
      {/* IM 渠道：按伙伴接待（platform → companionId 反向视图）/ Per-companion IM channels */}
      <RemoteConnectSection companionId={profile.companion_id} companionName={profile.name} />
      <AccessTokenSection
        companionId={profile.companion_id}
        companionName={profile.name}
        modelConfigured={status?.model_configured ?? null}
      />
    </div>
  );
};

export default RemoteTab;
