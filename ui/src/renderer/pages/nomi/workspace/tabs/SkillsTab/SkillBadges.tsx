/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import type { ICompanionSkill } from '@/common/adapter/ipcBridge';
import type { SkillEntry } from './unify';

/**
 * Quiet pills. The row already carries a name and a description; badges only
 * answer two questions — where did this skill come from, and is it live.
 */
const PILL = 'shrink-0 rd-full px-6px text-10px leading-16px font-500 whitespace-nowrap';

export const SkillSourceBadge: React.FC<{ entry: SkillEntry }> = ({ entry }) => {
  const { t } = useTranslation();
  return entry.kind === 'generated' ? (
    <span className={`${PILL} bg-[rgba(var(--primary-6),0.12)] text-primary-6`}>
      {t('nomi.skills.sourceGenerated', { defaultValue: '自动生成' })}
    </span>
  ) : (
    <span className={`${PILL} bg-fill-2 text-t-secondary`}>
      {t('nomi.skills.sourceCatalog', { defaultValue: '已配置' })}
    </span>
  );
};

export const SkillStatusBadge: React.FC<{ status: ICompanionSkill['status'] }> = ({ status }) => {
  const { t } = useTranslation();
  if (status === 'draft') {
    return (
      <span className={`${PILL} bg-[rgba(var(--warning-6),0.14)] text-[rgb(var(--warning-6))]`}>
        {t('nomi.skills.statusDraftLabel', { defaultValue: '草稿' })}
      </span>
    );
  }
  if (status === 'archived') {
    return (
      <span className={`${PILL} bg-fill-2 text-t-tertiary`}>
        {t('nomi.skills.statusArchived', { defaultValue: '已归档' })}
      </span>
    );
  }
  return (
    <span className={`${PILL} bg-[rgba(var(--success-6),0.14)] text-[rgb(var(--success-6))]`}>
      {t('nomi.skills.statusActive', { defaultValue: '已启用' })}
    </span>
  );
};

export const SkillMissingBadge: React.FC = () => {
  const { t } = useTranslation();
  return (
    <span className={`${PILL} bg-[rgba(var(--danger-6),0.12)] text-[rgb(var(--danger-6))]`}>
      {t('nomi.skills.configMissing', { defaultValue: '未安装' })}
    </span>
  );
};
