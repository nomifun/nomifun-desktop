/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Avatar, Typography } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import { resolveAgentLogo } from '@/renderer/utils/model/agentLogo';
import { resolveExtensionAssetUrl } from '@/renderer/utils/platform';

type DetectedAgent = {
  agent_type: string;
  backend?: string;
  agent_id?: string;
  icon?: string;
  name: string;
  isExtension?: boolean;
  avatar?: string;
};

type AgentCardProps = {
  agent: DetectedAgent;
};

const AgentCard: React.FC<AgentCardProps> = ({ agent }) => {
  const { t } = useTranslation();
  const extensionAvatar = resolveExtensionAssetUrl(agent.isExtension ? agent.avatar : undefined);
  const logo =
    extensionAvatar ||
    resolveAgentLogo({
      icon: agent.icon,
      backend: agent.backend || agent.agent_type,
      agentId: agent.agent_id,
      isExtension: agent.isExtension,
    });

  return (
    <div className='flex min-h-[112px] flex-col rounded-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] p-12px transition-colors hover:border-[var(--color-border-3)]'>
      <div className='mb-10px flex justify-center'>
        <Avatar size={40} shape='square' style={{ flexShrink: 0, backgroundColor: 'transparent' }}>
          {logo ? <img src={logo} alt={agent.name} className='h-full w-full object-contain' /> : '🤖'}
        </Avatar>
      </div>

      <div className='flex-1 text-center'>
        <Typography.Text className='block text-13px font-medium leading-18px line-clamp-2'>
          {agent.name}
        </Typography.Text>
        <Typography.Text className='mt-4px block text-11px text-t-secondary'>
          {t('settings.agentManagement.installed')}
        </Typography.Text>
      </div>

    </div>
  );
};

export default AgentCard;
