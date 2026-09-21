/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { getAgentLogo } from '@/renderer/utils/model/agentLogo';
import { iconColors } from '@/renderer/styles/colors';
import { Robot } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';

export type AgentBadgeProps = {
  /** Agent backend type */
  backend?: string;
  /** Display name for the agent */
  agent_name?: string;
  /** Custom agent logo (SVG path or emoji string) */
  agentLogo?: string;
  /** Whether the logo is an emoji */
  agentLogoIsEmoji?: boolean;
};

export type AgentIdentityBadgeProps = AgentBadgeProps & {
  /** The identity that owns the current conversation or generation request. */
  name?: string | null;
  /** Render the identity as a loading state while the binding is resolving. */
  loading?: boolean;
  /** Keep the label compact when it is rendered in a title bar or toolbar. */
  compact?: boolean;
  className?: string;
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

/**
 * A non-silent Agent identity marker for every product-owned Agent surface.
 *
 * AgentLogoIcon is intentionally kept as a low-level primitive for message
 * avatars. Entry points need the name as text as well: an icon alone does not
 * tell a user which Agent is going to receive the next request. Keeping this
 * marker in one component also gives tests and accessibility tooling a stable
 * `data-agent-identity` seam.
 */
export const AgentIdentityBadge: React.FC<AgentIdentityBadgeProps> = ({
  backend,
  agent_name,
  agentLogo,
  agentLogoIsEmoji,
  name,
  loading = false,
  compact = false,
  className = '',
}) => {
  const { t } = useTranslation();
  const displayName = loading
    ? t('agent.identity.loading', { defaultValue: 'Loading…' })
    : name?.trim() || agent_name?.trim() || t('agent.identity.unknown', { defaultValue: 'Unspecified' });
  const label = t('agent.identity.label', { defaultValue: 'Using Agent' });
  const ariaLabel = t('agent.identity.ariaLabel', {
    defaultValue: '{{label}}: {{name}}',
    label,
    name: displayName,
  });

  return (
    <span
      className={`inline-flex min-w-0 items-center gap-5px rounded-6px border border-solid border-[var(--color-border-2)] bg-[var(--color-fill-1)] px-7px py-3px text-12px leading-16px text-t-secondary ${className}`}
      data-agent-identity
      data-agent-name={displayName}
      data-agent-loading={loading || undefined}
      aria-label={ariaLabel}
      title={ariaLabel}
    >
      <AgentLogoIcon
        backend={backend}
        agent_name={displayName}
        agentLogo={agentLogo}
        agentLogoIsEmoji={agentLogoIsEmoji}
      />
      <span className={compact ? 'sr-only' : 'shrink-0'}>{label}</span>
      <strong className='min-w-0 truncate font-600 text-t-primary'>{displayName}</strong>
    </span>
  );
};
