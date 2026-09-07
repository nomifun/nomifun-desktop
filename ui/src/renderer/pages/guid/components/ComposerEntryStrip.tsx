/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { EveryUser } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import styles from '../index.module.css';

export interface ComposerEntryStripProps {
  /** Companion draft entry for the new Session. */
  onSummonCompanion?: () => void;
  summonedCompanionName?: string | null;
}

/**
 * The Guid composer only exposes per-session controls here. Agent authoring and
 * MiniApp lifecycle actions belong to their own product surfaces.
 */
const ComposerEntryStrip: React.FC<ComposerEntryStripProps> = ({
  onSummonCompanion,
  summonedCompanionName,
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

  return <div className={styles.entryStrip}>{summonEntry}</div>;
};

export default ComposerEntryStrip;
