/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { MessageOne, Plus } from '@icon-park/react';
import SegmentedTabs from '@/renderer/components/base/SegmentedTabs';
import SkillButton from './SkillButton';
import type { SkillSourceFilter } from './unify';

interface SkillsToolbarProps {
  filter: SkillSourceFilter;
  onFilterChange: (filter: SkillSourceFilter) => void;
  /** Drafts are waiting: the mined-skills segment gets the attention dot. */
  hasDrafts: boolean;
  /** Grants cannot be changed yet (no loaded profile to patch). */
  grantsDisabled: boolean;
  onLearnFromSession: () => void;
  onAddCapability: () => void;
}

const SkillsToolbar: React.FC<SkillsToolbarProps> = ({
  filter,
  onFilterChange,
  hasDrafts,
  grantsDisabled,
  onLearnFromSession,
  onAddCapability,
}) => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-wrap items-center gap-10px'>
      <SegmentedTabs
        size='sm'
        activeKey={filter}
        onChange={(key) => onFilterChange(key as SkillSourceFilter)}
        items={[
          { key: 'all', label: t('nomi.skills.filterAll', { defaultValue: '全部' }) },
          {
            key: 'generated',
            label: t('nomi.skills.sourceGenerated', { defaultValue: '自动生成' }),
            dot: hasDrafts,
          },
          { key: 'catalog', label: t('nomi.skills.sourceCatalog', { defaultValue: '已配置' }) },
        ]}
      />
      <div className='ml-auto flex items-center gap-8px'>
        <SkillButton
          icon={<MessageOne theme='outline' size='12' fill='currentColor' strokeWidth={3} />}
          onClick={onLearnFromSession}
        >
          {t('nomi.skills.learnFromSession', { defaultValue: '从会话学习' })}
        </SkillButton>
        <SkillButton
          tone='primary'
          disabled={grantsDisabled}
          icon={<Plus theme='outline' size='12' fill='currentColor' strokeWidth={4} />}
          onClick={onAddCapability}
        >
          {t('nomi.skills.addCapability', { defaultValue: '添加能力' })}
        </SkillButton>
      </div>
    </div>
  );
};

export default SkillsToolbar;
