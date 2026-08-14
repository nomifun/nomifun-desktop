/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { Robot } from '@icon-park/react';
import type { Preset } from '@/common/types/agent/presetTypes';
import { CUSTOM_AVATAR_IMAGE_MAP } from '@/renderer/pages/guid/constants';
import { getAgentLogo } from '@/renderer/utils/model/agentLogo';
import type { AgentMetadata } from '@/renderer/utils/model/agentTypes';
import {
  isEmoji,
  resolvePresetAvatarImageSrc,
  resolvePresetCatalogName,
} from '@/renderer/utils/model/presetPresentation';
import { resolveCronAgentDisplayName } from './cronAgentSelection';

const Identity: React.FC<{
  name: string;
  avatar?: string;
  fallbackLogo?: string | null;
  statusLabel?: string;
  compact?: boolean;
}> = ({ name, avatar, fallbackLogo, statusLabel, compact = false }) => {
  const avatarValue = avatar?.trim();
  const resolvedAvatar = resolvePresetAvatarImageSrc(avatarValue, CUSTOM_AVATAR_IMAGE_MAP);
  const [failedImages, setFailedImages] = useState<ReadonlySet<string>>(() => new Set());
  const avatarImage = resolvedAvatar && !failedImages.has(resolvedAvatar) ? resolvedAvatar : undefined;
  // A mapped decorative image may fail offline; retain the original Emoji as
  // the next fallback instead of dropping straight to the generic Robot.
  const avatarEmoji = avatarValue && !avatarImage && isEmoji(avatarValue) ? avatarValue : undefined;
  const fallbackImage =
    !avatarImage && !avatarEmoji && fallbackLogo && !failedImages.has(fallbackLogo) ? fallbackLogo : undefined;
  const image = avatarImage || fallbackImage;

  return (
    <div className='flex min-w-0 items-center gap-8px' title={statusLabel ? `${name} · ${statusLabel}` : name}>
      {image ? (
        <img
          src={image}
          alt=''
          className='h-16px w-16px shrink-0 object-contain'
          onError={() => setFailedImages((previous) => new Set(previous).add(image))}
        />
      ) : avatarEmoji ? (
        <span className='shrink-0 text-14px leading-16px' aria-hidden='true'>
          {avatarEmoji}
        </span>
      ) : (
        <Robot size='16' className='shrink-0' aria-hidden='true' />
      )}
      <span className={compact ? 'min-w-0 flex-1 truncate' : 'flex min-w-0 flex-1 flex-col'}>
        <span className='truncate'>{name}</span>
        {!compact && statusLabel && (
          <span className='truncate text-12px leading-16px text-t-tertiary'>{statusLabel}</span>
        )}
        {compact && statusLabel && (
          <span
            style={{
              position: 'absolute',
              width: 1,
              height: 1,
              padding: 0,
              margin: -1,
              overflow: 'hidden',
              clip: 'rect(0, 0, 0, 0)',
              whiteSpace: 'nowrap',
              border: 0,
            }}
          >
            {statusLabel}
          </span>
        )}
      </span>
    </div>
  );
};

export const CronAgentOptionIdentity: React.FC<{
  agent: AgentMetadata;
  language: string;
  statusLabel?: string;
  compact?: boolean;
}> = ({ agent, language, statusLabel, compact }) => (
  <Identity
    name={resolveCronAgentDisplayName(agent, language)}
    avatar={agent.icon}
    fallbackLogo={getAgentLogo(agent.backend || agent.agent_type)}
    statusLabel={statusLabel}
    compact={compact}
  />
);

export const CronPresetOptionIdentity: React.FC<{
  preset: Preset;
  language: string;
  nameOverride?: string;
  statusLabel?: string;
  compact?: boolean;
}> = ({ preset, language, nameOverride, statusLabel, compact }) => (
  <Identity
    name={nameOverride?.trim() || resolvePresetCatalogName(preset, language)}
    avatar={preset.avatar}
    statusLabel={statusLabel}
    compact={compact}
  />
);

export const CronUnavailableAgentIdentity: React.FC<{
  name: string;
  statusLabel?: string;
  compact?: boolean;
}> = ({ name, statusLabel, compact }) => <Identity name={name} statusLabel={statusLabel} compact={compact} />;
