/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTechnicalCapability } from '@/common/config/storage';
import { Alert, Button, Tag } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import styles from '../index.module.css';

type GuidModelCompatibilityNoticeProps = {
  modelLabel: string;
  providerLabel: string;
  missingCapabilities: readonly ModelTechnicalCapability[];
  compatibleModelCount: number;
  canConfigureCurrentModel: boolean;
  onChooseCompatibleModel: () => void;
  onOpenModelConfiguration: () => void;
};

/** A launch blocker should diagnose the exact gap and offer a direct recovery path. */
const GuidModelCompatibilityNotice: React.FC<GuidModelCompatibilityNoticeProps> = ({
  modelLabel,
  providerLabel,
  missingCapabilities,
  compatibleModelCount,
  canConfigureCurrentModel,
  onChooseCompatibleModel,
  onOpenModelConfiguration,
}) => {
  const { t } = useTranslation();

  return (
    <Alert
      type='warning'
      showIcon
      title={t('guid.agentEntries.modelCompatibility.title')}
      className={`${styles.guidPresetCapabilityError} ${styles.modelCompatibilityAlert}`}
      content={
        <div className={styles.modelCompatibilityBody}>
          <p className={styles.modelCompatibilitySummary}>
            {t('guid.agentEntries.modelCompatibility.summary', { model: modelLabel })}
          </p>

          <dl className={styles.modelCompatibilityFacts}>
            <div className={styles.modelCompatibilityFact}>
              <dt>{t('guid.agentEntries.modelCompatibility.currentModel')}</dt>
              <dd>
                <strong>{modelLabel}</strong>
                <span className={styles.modelCompatibilityProvider}>· {providerLabel}</span>
              </dd>
            </div>
            <div className={styles.modelCompatibilityFact}>
              <dt>{t('guid.agentEntries.modelCompatibility.missingCapabilities')}</dt>
              <dd className={styles.modelCompatibilityTags}>
                {missingCapabilities.map((capability) => (
                  <Tag key={capability} size='small' color='orange'>
                    {t(`settings.modelTechnicalCapability.${capability}`)}
                  </Tag>
                ))}
              </dd>
            </div>
          </dl>

          <p className={styles.modelCompatibilityPreserved}>
            {t('guid.agentEntries.modelCompatibility.preserved')}
          </p>

          <div className={styles.modelCompatibilityActions}>
            <Button
              type='primary'
              size='small'
              disabled={compatibleModelCount === 0}
              onClick={onChooseCompatibleModel}
            >
              {t('guid.agentEntries.modelCompatibility.chooseCompatible', {
                count: compatibleModelCount,
              })}
            </Button>
            <Button type='text' size='small' onClick={onOpenModelConfiguration}>
              {t(
                canConfigureCurrentModel
                  ? 'guid.agentEntries.modelCompatibility.configureCurrent'
                  : 'guid.agentEntries.modelCompatibility.viewChatModels',
              )}
            </Button>
            {compatibleModelCount === 0 && (
              <span className={styles.modelCompatibilityNoAlternative}>
                {t('guid.agentEntries.modelCompatibility.noCompatibleModel')}
              </span>
            )}
          </div>
        </div>
      }
    />
  );
};

export default GuidModelCompatibilityNotice;
