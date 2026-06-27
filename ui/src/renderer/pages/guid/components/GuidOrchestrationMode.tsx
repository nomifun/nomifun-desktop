/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import SegmentedTabs from '@/renderer/components/base/SegmentedTabs';
import type { GuidModelSelectionMode } from '../hooks/useGuidModelSelection';
import { Tooltip } from '@arco-design/web-react';
import { Robot } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';

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
 * Uses the shared {@link SegmentedTabs} pill control (soft `bg-fill-2` track,
 * subtle `primary-1` tint on the active segment) so it reads as a tasteful,
 * theme-aware toggle rather than a loud full-saturation fill. auto / range both
 * mean "multi-agent orchestration is ON" and carry a Robot glyph so the active
 * state communicates the intent at a glance.
 *
 * Rendered by GuidPage only when the active agent is Nomi.
 */
const GuidOrchestrationMode: React.FC<GuidOrchestrationModeProps> = ({ selectionMode, setSelectionMode }) => {
  const { t } = useTranslation();

  return (
    <Tooltip content={t('guid.orchestration.tooltip')} position='top'>
      <SegmentedTabs
        size='sm'
        className='mr-4px'
        activeKey={selectionMode}
        onChange={(key) => setSelectionMode(key as GuidModelSelectionMode)}
        items={[
          { key: 'single', label: t('guid.orchestration.modeSingle') },
          { key: 'auto', label: t('guid.orchestration.modeAuto'), icon: <Robot theme='outline' size='13' /> },
          { key: 'range', label: t('guid.orchestration.modeRange'), icon: <Robot theme='outline' size='13' /> },
        ]}
      />
    </Tooltip>
  );
};

export default GuidOrchestrationMode;
