/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { AgentPresetDocument } from '@/common/types/agentPlatform';
import { createDefaultIdmmConfig } from '@/common/types/idmm';
import IdmmControl from '@/renderer/pages/conversation/components/IdmmControl';
import React from 'react';
import { useTranslation } from 'react-i18next';
import styles from './AgentSettingsPage.module.css';

type Props = {
  document: AgentPresetDocument;
  disabled: boolean;
  onChange: (document: AgentPresetDocument) => void;
  idPrefix: 'agent' | 'template';
};

const AgentRuntimePolicyPanel: React.FC<Props> = ({
  document,
  disabled,
  onChange,
  idPrefix,
}) => {
  const { t } = useTranslation();
  const idmmPolicy = document.runtime_policy?.idmm ?? createDefaultIdmmConfig();

  return (
    <div
      role='tabpanel'
      id={`${idPrefix}-panel-runtime`}
      aria-labelledby={`${idPrefix}-tab-runtime`}
    >
      <section className={styles.section} id={`${idPrefix}-settings-runtime-policy`}>
        <div className={styles.sectionHeading}>
          <div>
            <h3>{t('agentSettings.runtimePolicy.title')}</h3>
            <p>{t('agentSettings.runtimePolicy.hint')}</p>
          </div>
          <span className={styles.runtimePolicyBoundary}>
            {t('agentSettings.runtimePolicy.noCapabilityGrant')}
          </span>
        </div>
        <ol
          className={styles.runtimePolicyFlow}
          aria-label={t('agentSettings.runtimePolicy.flowLabel')}
        >
          {(['agent', 'session', 'override'] as const).map((step, index) => (
            <li key={step} className={styles.runtimePolicyFlowStep}>
              <span className={styles.runtimePolicyStepNumber}>{index + 1}</span>
              <span>
                <strong>{t(`agentSettings.runtimePolicy.flow.${step}.title`)}</strong>
                <small>{t(`agentSettings.runtimePolicy.flow.${step}.description`)}</small>
              </span>
            </li>
          ))}
        </ol>
        <div className={styles.runtimePolicyEditor}>
          <IdmmControl
            presentation='embedded'
            draft={{
              value: idmmPolicy,
              onChange: (idmm) =>
                onChange({
                  ...document,
                  runtime_policy: { ...document.runtime_policy, idmm },
                }),
            }}
            disabledReason={disabled ? t('agentSettings.runtimePolicy.busy') : undefined}
            applyNote={t('agentSettings.runtimePolicy.applyNote')}
          />
        </div>
      </section>
    </div>
  );
};

export default AgentRuntimePolicyPanel;
