/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { iconColors } from '@/renderer/styles/colors';
import type { GuidModelSelectionMode } from '../hooks/useGuidModelSelection';
import { Radio, Tooltip } from '@arco-design/web-react';
import { Robot } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';
import styles from '../index.module.css';

type GuidOrchestrationModeProps = {
  /** Active tri-state mode (single / auto / range). */
  selectionMode: GuidModelSelectionMode;
  /** Switch the active mode. */
  setSelectionMode: (mode: GuidModelSelectionMode) => void;
};

/**
 * Visible orchestration-mode switch for the 会话 input bar.
 *
 * Surfaces the single / auto / range tri-state as a one-click segmented control
 * sitting next to the model selector — previously this lived buried inside the
 * model dropdown, which made multi-agent orchestration hard to discover. It is
 * the single source of truth for `selectionMode`; the dropdown body stays
 * mode-aware but no longer carries its own switch.
 *
 * Rendered by GuidPage only when the active agent is Nomi.
 */
const GuidOrchestrationMode: React.FC<GuidOrchestrationModeProps> = ({ selectionMode, setSelectionMode }) => {
  const { t } = useTranslation();

  // auto / range both mean "multi-agent orchestration is ON" — give a subtle
  // primary tint + Robot glyph so the active state reads at a glance.
  const orchestrationOn = selectionMode !== 'single';

  return (
    <Tooltip content={t('guid.orchestration.tooltip')} position='top'>
      <div className={`${styles.orchestrationMode} ${orchestrationOn ? styles.orchestrationModeOn : ''}`}>
        {orchestrationOn ? (
          <Robot theme='outline' size='13' fill={iconColors.brand} className={styles.orchestrationModeIcon} />
        ) : (
          <span className={styles.orchestrationModeLabel}>{t('guid.orchestration.label')}</span>
        )}
        <Radio.Group
          type='button'
          size='small'
          value={selectionMode}
          onChange={(mode: GuidModelSelectionMode) => setSelectionMode(mode)}
        >
          <Radio value='single'>{t('guid.orchestration.modeSingle')}</Radio>
          <Radio value='auto'>{t('guid.orchestration.modeAuto')}</Radio>
          <Radio value='range'>{t('guid.orchestration.modeRange')}</Radio>
        </Radio.Group>
      </div>
    </Tooltip>
  );
};

export default GuidOrchestrationMode;
