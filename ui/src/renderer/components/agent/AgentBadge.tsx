/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { getAgentLogo } from '@/renderer/utils/model/agentLogo';
import { iconColors } from '@/renderer/styles/colors';
import { Robot } from '@icon-park/react';
import React from 'react';
import type { PresetReference } from '@/common/types/agent/presetTypes';

export type AgentBadgeProps = {
  /** Agent backend type */
  backend?: string;
  /** Display name for the agent */
  agent_name?: string;
  /** Custom agent logo (SVG path or emoji string) */
  agentLogo?: string;
  /** Whether the logo is an emoji */
  agentLogoIsEmoji?: boolean;
  /** Preset lineage for callers that expose configuration details. */
  presetId?: PresetReference;
};

/** Render agent logo from custom logo, backend logo, or fallback Robot icon */
export const AgentLogoIcon: React.FC<
  Pick<AgentBadgeProps, 'backend' | 'agentLogo' | 'agentLogoIsEmoji' | 'agent_name'>
> = ({ backend, agentLogo, agentLogoIsEmoji, agent_name }) => {
  const logoContent = (() => {
    if (agentLogo) {
      if (agentLogoIsEmoji) {
        return <span className='text-14px leading-none'>{agentLogo}</span>;
      }
      return (
        <img src={agentLogo} alt={`${agent_name || 'agent'} logo`} className='block w-16px h-16px object-contain' />
      );
    }
    const logo = getAgentLogo(backend);
    if (logo) {
      return <img src={logo} alt={`${backend} logo`} className='block w-16px h-16px object-contain' />;
    }
    return <Robot theme='outline' size={16} fill={iconColors.primary} />;
  })();

  return (
    <span className='inline-flex w-16px h-16px items-center justify-center shrink-0 leading-none'>{logoContent}</span>
  );
};
