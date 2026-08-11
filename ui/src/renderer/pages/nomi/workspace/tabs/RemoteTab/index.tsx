/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Spin } from '@arco-design/web-react';
import React, { useEffect, useRef, useState } from 'react';

import RemoteConnectSection from './RemoteConnectSection';
import type { WorkspaceTabProps } from '../../types';
import AccessTokenSection from './AccessTokenSection';
import RobotConnectSection from './RobotConnectSection';
import { usePairingAttention } from './usePairingAttention';

/**
 * 远程控制 tab：从桌面应用之外触达这只伙伴的三条路径 —— IM 渠道（谁来接待）、
 * 实体机器人（硬件设备用它的人格说话）与远程访问令牌（外部客户端以它的身份接入）。
 *
 * Remote control tab. Three sections, one idea each: the IM channel bots this
 * companion greets on, the physical robots bound to it, and the per-companion
 * access token for external MCP / REST clients. Attention (the strip dot) means
 * a pairing request is waiting for approval, or a bound robot cannot reach this
 * machine because LAN access is off.
 */
const RemoteTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { profile, status } = companion;

  const pendingPairings = usePairingAttention(profile ? companionId : null);
  const [robotAttention, setRobotAttention] = useState(false);

  const attentionRef = useRef(onAttentionChange);
  attentionRef.current = onAttentionChange;
  useEffect(() => {
    attentionRef.current?.(pendingPairings > 0 || robotAttention);
  }, [pendingPairings, robotAttention]);
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
      {/* 实体机器人：绑到这只伙伴的硬件设备 / Physical robots bound to this companion */}
      <RobotConnectSection
        companionId={profile.companion_id}
        companionName={profile.name}
        onAttentionChange={setRobotAttention}
      />
      <AccessTokenSection
        companionId={profile.companion_id}
        companionName={profile.name}
        modelConfigured={status?.model_configured ?? null}
      />
    </div>
  );
};

export default RemoteTab;
