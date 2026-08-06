/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Brain } from '@icon-park/react';

interface MemoryListEmptyProps {
  /** A filtered empty result invites clearing filters, not creating a memory. */
  filtered: boolean;
  onAdd: () => void;
}

/**
 * Zero-state of the memory list: soft circular badge, title, one calm line, one
 * round CTA. Nothing else — an empty list is an invitation, not an error.
 */
const MemoryListEmpty: React.FC<MemoryListEmptyProps> = ({ filtered, onAdd }) => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col items-center justify-center gap-14px px-24px py-64px text-center'>
      <div
        className='flex items-center justify-center rd-full bg-fill-2'
        style={{ width: 72, height: 72, color: 'rgb(var(--primary-6))' }}
      >
        <Brain theme='outline' size='32' fill='currentColor' strokeWidth={3} />
      </div>
      <div className='flex flex-col items-center gap-6px'>
        <span className='text-16px font-500 text-t-primary'>
          {filtered
            ? t('nomi.memory.emptyFilteredTitle', { defaultValue: '没有符合条件的记忆' })
            : t('nomi.memory.emptyTitle', { defaultValue: '还没有记忆' })}
        </span>
        <span className='max-w-360px text-13px leading-20px text-t-tertiary'>
          {filtered
            ? t('nomi.memory.emptyFilteredDesc', { defaultValue: '换个关键词或放宽筛选条件再看看。' })
            : t('nomi.memory.emptyDesc', { defaultValue: '开启数据采集并运行一次学习，或先手动记下一件事。' })}
        </span>
      </div>
      {!filtered && (
        <div
          role='button'
          tabIndex={0}
          onClick={onAdd}
          onKeyDown={(event) => {
            if (event.key === 'Enter' || event.key === ' ') {
              event.preventDefault();
              onAdd();
            }
          }}
          className='mt-4px inline-flex cursor-pointer select-none items-center rd-full bg-[rgba(var(--primary-6),0.12)] px-18px py-9px text-13px font-700 leading-none text-[var(--color-text-1)] shadow-[0_6px_18px_rgba(var(--primary-6),0.14)] transition-colors hover:bg-[rgba(var(--primary-6),0.18)]'
        >
          {t('nomi.memories.add', { defaultValue: '添加记忆' })}
        </div>
      )}
    </div>
  );
};

export default MemoryListEmpty;
