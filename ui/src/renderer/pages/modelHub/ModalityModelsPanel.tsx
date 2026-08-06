/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import type { ModalityKey } from './modalityModels';

export interface ModalityModelsPanelProps {
  modality: ModalityKey;
  icon: React.ReactNode;
  titleKey: I18nKey;
  subtitleKey: I18nKey;
}

/** 模态分区通用面板（Task 10 填充完整行渲染）。 */
const ModalityModelsPanel: React.FC<ModalityModelsPanelProps> = ({ icon, titleKey, subtitleKey }) => {
  const { t } = useTranslation();
  return (
    <div className='flex min-h-0 flex-col rd-16px bg-2 px-24px py-16px'>
      <header className='flex items-center gap-9px border-b border-b-solid border-[var(--color-border-2)] pb-14px'>
        <span className='size-30px shrink-0 flex items-center justify-center rd-9px bg-primary-1 text-primary-6'>
          {icon}
        </span>
        <div className='min-w-0'>
          <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>{t(titleKey)}</h2>
          <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>{t(subtitleKey)}</p>
        </div>
      </header>
    </div>
  );
};

export default ModalityModelsPanel;
