/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ApplicationOne, EveryUser } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import styles from '../index.module.css';

export interface ComposerEntryStripProps {
  /** Companion draft entry for the new Session. */
  onSummonCompanion?: () => void;
  summonedCompanionName?: string | null;
  /** Mini-app entry. Omit to hide this capability on a surface. */
  onCreateMiniApp?: () => void;
  miniAppActive?: boolean;
  onDismissMiniApp?: () => void;
}

/**
 * The Guid composer only exposes per-session controls here. Agent
 * authoring is intentionally not embedded in the quick-start surface.
 */
const ComposerEntryStrip: React.FC<ComposerEntryStripProps> = ({
  onSummonCompanion,
  summonedCompanionName,
  onCreateMiniApp,
  miniAppActive = false,
  onDismissMiniApp,
}) => {
  const { t } = useTranslation();
  const summonEntry = onSummonCompanion ? (
    <button
      type='button'
      className={`${styles.entryButton} ${styles.entryButtonInteractive}`}
      onClick={onSummonCompanion}
      aria-label={t('conversation.summon.buttonTooltip')}
      data-testid='guid-summon-entry'
    >
      <EveryUser theme='outline' size={15} fill='currentColor' />
      <span className={styles.entryButtonText}>
        {summonedCompanionName || t('conversation.summon.button')}
      </span>
    </button>
  ) : null;

  const miniAppEntry = !onCreateMiniApp ? null : miniAppActive ? (
    <span
      className={`${styles.entryButton} ${styles.entryButtonActive} ${styles.entryPersonaButton}`}
      data-testid='guid-miniapp-token'
    >
      <span className={styles.entryAvatar}>
        <ApplicationOne theme='outline' size={16} fill='currentColor' />
      </span>
      <span className={styles.entryButtonText}>{t('miniApps.composer.activeLabel')}</span>
      <button
        type='button'
        className={styles.entryDismiss}
        onClick={onDismissMiniApp}
        aria-label={t('miniApps.composer.dismiss')}
      >
        ×
      </button>
    </span>
  ) : (
    <button
      type='button'
      className={`${styles.entryButton} ${styles.entryButtonInteractive}`}
      onClick={onCreateMiniApp}
      aria-label={t('miniApps.composer.entry')}
      data-testid='guid-miniapp-entry'
    >
      <ApplicationOne theme='outline' size={15} fill='currentColor' />
      <span className={styles.entryButtonText}>{t('miniApps.composer.entry')}</span>
    </button>
  );

  return (
    <div className={styles.entryStrip}>
      {summonEntry}
      {miniAppEntry}
    </div>
  );
};

export default ComposerEntryStrip;
