/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { getBaseUrl } from '@/common/adapter/httpBridge';
import CustomFigure from './characters/CustomFigure';
import { CUSTOM_CHARACTER_ID, getCharacter } from './characters';
import { customFigureUrlOf } from './characters/customMeta';
import type { CustomFigureMeta, CompanionActivity, CompanionMood } from './characters';
import type { CompanionId } from '@/common/types/ids';

interface CompanionAvatarProps {
  /** Character id from companion config (appearance.character); falls back to default. */
  character?: string | null;
  mood: CompanionMood;
  activity: CompanionActivity;
  size?: number;
  /** Required for character==='custom': which companion's figure to load. */
  companionId?: CompanionId;
  /** Required for character==='custom': figure metadata from the companion profile. */
  customFigure?: CustomFigureMeta | null;
  /** 自定义立绘的命中元素（index.tsx 的 data-companion-hit wrapper），用于挂 alpha 掩码。 */
  figureHitRef?: React.RefObject<HTMLElement | null>;
}

/** Renders the configured companion character. The single entry point every page uses. */
const CompanionAvatar: React.FC<CompanionAvatarProps> = ({ character, mood, activity, size, companionId, customFigure, figureHitRef }) => {
  if (character === CUSTOM_CHARACTER_ID && companionId && customFigure) {
    const src = customFigureUrlOf(getBaseUrl(), companionId, customFigure);
    return (
      <CustomFigure
        key={src}
        src={src}
        aspect={customFigure.aspect}
        headBox={customFigure.headBox}
        mood={mood}
        activity={activity}
        size={size}
        hitRef={figureHitRef}
      />
    );
  }
  const { Component } = getCharacter(character);
  return <Component mood={mood} activity={activity} size={size} />;
};

export default CompanionAvatar;
