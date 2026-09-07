/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ApplicationOne } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import styles from '../index.module.css';

export interface ComposerEntryStripProps {
  /** The current Agent and its selection menu. */
  agentSelector?: React.ReactNode;
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
  agentSelector,
  onCreateMiniApp,
  miniAppActive = false,
  onDismissMiniApp,
}) => {
  const { t } = useTranslation();
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

  if (!miniAppEntry && !agentSelector) return null;

  return (
    <div className={styles.entryStrip}>
      {agentSelector && <div className={styles.entryAgentSelector}>{agentSelector}</div>}
      {miniAppEntry}
    </div>
  );
};

export default ComposerEntryStrip;
