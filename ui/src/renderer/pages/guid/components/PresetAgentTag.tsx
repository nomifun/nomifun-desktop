/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CUSTOM_AVATAR_IMAGE_MAP } from '../constants';
import type { AvailableAgent } from '../types';
import type { Preset } from '@/common/types/agent/presetTypes';
import { IconClose } from '@arco-design/web-react/icon';
import { Down, Robot } from '@icon-park/react';
import React from 'react';
import {
  isEmoji,
  resolvePresetAvatarImageSrc,
  resolvePresetCatalogName,
} from '@/renderer/utils/model/presetPresentation';
import { Dropdown, Menu } from '@arco-design/web-react';
import styles from '../index.module.css';

export type AgentSwitcherItem = {
  key: string;
  label: string;
  isCurrent: boolean;
};

type PresetAgentTagProps = {
  agentInfo: AvailableAgent;
  /** Backend-merged preset catalog used to resolve a localized name. */
  presets: Preset[];
  localeKey: string;
  onClose: () => void;
  agentLogo?: string | null;
  agentSwitcherItems?: AgentSwitcherItem[];
  onAgentSwitch?: (key: string) => void;
};

const PresetAgentTag: React.FC<PresetAgentTagProps> = ({
  agentInfo,
  presets,
  localeKey,
  onClose,
  agentLogo,
  agentSwitcherItems,
  onAgentSwitch,
}) => {
  const avatarValue = agentInfo.avatar?.trim();
  const avatarImage = resolvePresetAvatarImageSrc(avatarValue, CUSTOM_AVATAR_IMAGE_MAP);
  const avatarEmoji = avatarValue && !avatarImage && isEmoji(avatarValue) ? avatarValue : undefined;
  const preset = presets.find((item) => item.preset_id === agentInfo.preset_id);
  const name = preset ? resolvePresetCatalogName(preset, localeKey) : agentInfo.name;

  const hasSwitcher = Boolean(agentSwitcherItems && agentSwitcherItems.length > 0 && onAgentSwitch);

  const droplist = hasSwitcher ? (
    <Menu onClickMenuItem={(key) => onAgentSwitch?.(key)}>
      {agentSwitcherItems!.map((item) => (
        <Menu.Item key={item.key}>
          <div className='flex items-center justify-between gap-12px min-w-120px'>
            <span>{item.label}</span>
            {item.isCurrent ? <span>✓</span> : null}
          </div>
        </Menu.Item>
      ))}
    </Menu>
  ) : null;

  const mainBody = (
    <div className={styles.presetAgentTagMain}>
      {agentLogo ? (
        <>
          <img src={agentLogo} alt='' width={15} height={15} className={styles.presetAgentTagAgentLogo} />
          {hasSwitcher ? (
            <span className={styles.presetAgentTagChevron} aria-hidden='true'>
              <Down theme='outline' size={12} fill='currentColor' />
            </span>
          ) : null}
          <span className={styles.presetAgentTagInnerDivider} aria-hidden='true' />
        </>
      ) : hasSwitcher ? (
        <span className={styles.presetAgentTagChevron} aria-hidden='true'>
          <Down theme='outline' size={12} fill='currentColor' />
        </span>
      ) : null}
      {avatarImage ? (
        <img src={avatarImage} alt='' width={15} height={15} style={{ objectFit: 'contain', flexShrink: 0 }} />
      ) : avatarEmoji ? (
        <span style={{ fontSize: 14, lineHeight: '15px', flexShrink: 0 }}>{avatarEmoji}</span>
      ) : (
        <Robot theme='outline' size={15} style={{ flexShrink: 0 }} />
      )}
      <span className={styles.presetAgentTagName}>{name}</span>
    </div>
  );

  return (
    <div className={styles.presetAgentTag}>
      {/* Left: agent logo | avatar + name + ▾ — whole area triggers agent switcher dropdown */}
      {hasSwitcher ? (
        <Dropdown trigger='click' position='bl' droplist={droplist}>
          {mainBody}
        </Dropdown>
      ) : (
        mainBody
      )}

      {/* Divider */}
      <span className={styles.presetAgentTagDivider} aria-hidden='true' />

      {/* Right: always × to close */}
      <div
        className={styles.presetAgentTagClose}
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
      >
        <IconClose style={{ fontSize: 12, color: 'var(--color-text-3)' }} />
      </div>
    </div>
  );
};

export default PresetAgentTag;
