/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Puzzle } from '@icon-park/react';
import SkillButton from './SkillButton';

/**
 * Zero state: this companion has no skills at all. One invitation — grant a
 * capability from the library; everything else (mining from real work) happens
 * on its own, which the description says instead of adding a second CTA.
 */
const SkillsEmptyState: React.FC<{ addDisabled?: boolean; onAddCapability: () => void }> = ({
  addDisabled = false,
  onAddCapability,
}) => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col items-center justify-center gap-14px px-24px py-64px text-center'>
      <div
        className='flex items-center justify-center rounded-full'
        style={{ width: 72, height: 72, background: 'var(--color-fill-2)', color: 'rgb(var(--primary-6))' }}
      >
        <Puzzle theme='outline' size='32' fill='currentColor' strokeWidth={3} />
      </div>
      <div className='flex flex-col items-center gap-6px'>
        <span className='text-16px font-500 text-[var(--color-text-1)]'>
          {t('nomi.skills.emptyTitle', { defaultValue: '还没有技能' })}
        </span>
        <span className='max-w-360px text-13px leading-20px text-[var(--color-text-3)]'>
          {t('nomi.skills.emptyDesc', {
            defaultValue: '先从技能库授予一个能力；之后伙伴在真实工作里还会自己沉淀新技能。',
          })}
        </span>
      </div>
      <SkillButton tone='primary' size='md' className='mt-4px' disabled={addDisabled} onClick={onAddCapability}>
        {t('nomi.skills.addCapability', { defaultValue: '添加能力' })}
      </SkillButton>
    </div>
  );
};

export default SkillsEmptyState;
