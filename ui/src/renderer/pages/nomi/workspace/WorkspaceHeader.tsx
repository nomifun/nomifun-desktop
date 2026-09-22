/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@arco-design/web-react';
import { ApplicationMenu } from '@icon-park/react';
import CompanionAvatar from '@renderer/pages/companion/CompanionAvatar';
import { customFigureMetaOf } from '@renderer/pages/companion/characters/customMeta';
import type { CompanionMood } from '@renderer/pages/companion/characters';
import type { CompanionHandle } from './types';
import styles from './WorkspaceHeader.module.css';

export type CompanionWorkspaceMode = 'cohabit' | 'manage';

interface Props {
  companion: CompanionHandle;
  mode: CompanionWorkspaceMode;
  onModeChange: (mode: CompanionWorkspaceMode) => void;
  onOpenQuickWindow: () => void;
}

/** Stable identity and the only first-level choice inside the companion product. */
const WorkspaceHeader: React.FC<Props> = ({ companion, mode, onModeChange, onOpenQuickWindow }) => {
  const { t } = useTranslation();
  const { profile, status } = companion;

  return (
    <header className={styles.header}>
      <div className={styles.identity}>
        {profile && (
          <CompanionAvatar
            character={profile.character}
            companionId={profile.companion_id}
            customFigure={customFigureMetaOf(profile)}
            mood={(status?.mood as CompanionMood) || 'content'}
            activity='idle'
            size={48}
          />
        )}
        <div className={styles.identityCopy}>
          <div className={styles.name}>{profile?.name ?? ''}</div>
          {status && (
            <div className={styles.meta}>
              <span>Lv {status.level} · {t(`nomi.levels.l${Math.min(status.level, 5)}`)}</span>
              <span className={styles.moodDot} aria-hidden='true' />
              <span>{t(`nomi.moods.${status.mood}`, { defaultValue: status.mood })}</span>
            </div>
          )}
        </div>
      </div>

      <div className={styles.modeSwitch} role='group' aria-label={t('nomi.workspace.modeLabel', { defaultValue: '伙伴模式' })}>
        <button
          type='button'
          aria-pressed={mode === 'cohabit'}
          onClick={() => onModeChange('cohabit')}
        >
          {t('nomi.workspace.cohabit', { defaultValue: '相处' })}
        </button>
        <button
          type='button'
          aria-pressed={mode === 'manage'}
          onClick={() => onModeChange('manage')}
        >
          {t('nomi.workspace.manage', { defaultValue: '管理' })}
        </button>
      </div>

      <Button
        className={styles.quickWindowButton}
        shape='round'
        size='small'
        icon={<ApplicationMenu theme='outline' size='14' fill='currentColor' />}
        onClick={onOpenQuickWindow}
      >
        {t('nomi.workspace.quickWindow', { defaultValue: '桌面快捷窗' })}
      </Button>
    </header>
  );
};

export default WorkspaceHeader;
