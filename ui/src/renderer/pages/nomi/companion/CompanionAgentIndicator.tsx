/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Robot } from '@icon-park/react';
import classNames from 'classnames';
import React from 'react';
import { useTranslation } from 'react-i18next';

type Props = {
  surface?: 'composer' | 'settings';
};

/** Read-only identity marker for the product-owned Companion Agent. */
const CompanionAgentIndicator: React.FC<Props> = ({ surface = 'composer' }) => {
  const { t } = useTranslation();
  const label = t('agentSettings.template.companion.default.name');

  return (
    <span
      data-testid='companion-agent-indicator'
      data-readonly='true'
      aria-label={t('nomi.chat.fixedAgentAria', { agent: label })}
      title={t('nomi.chat.fixedAgentHint')}
      className={classNames(
        'inline-flex min-w-0 items-center gap-6px rd-full text-12px font-500 text-t-secondary select-none',
        surface === 'composer'
          ? 'sendbox-model-btn header-model-btn nomi-sendbox-agent-btn h-28px px-10px'
          : 'h-28px px-10px bg-fill-2'
      )}
    >
      <Robot theme='outline' size={14} className='shrink-0' />
      <span className='sendbox-responsive-label truncate'>{label}</span>
    </span>
  );
};

export default CompanionAgentIndicator;
