/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { BookOne, Connect, Shield } from '@icon-park/react';
import type { ICompanionWithStatus } from '@/common/adapter/ipcBridge';
import CompanionAvatar from '@renderer/pages/companion/CompanionAvatar';
import { customFigureMetaOf } from '@renderer/pages/companion/characters/customMeta';
import type { CompanionMood } from '@renderer/pages/companion/characters';
import type { EmployeeStats } from './useOutbound';

interface Props {
  employee: ICompanionWithStatus;
  stats: EmployeeStats;
  onOpen: () => void;
}

/** One compact stat tile inside a card (icon + count + label). */
const StatTile: React.FC<{ icon: React.ReactNode; count: number; label: string; active?: boolean }> = ({
  icon,
  count,
  label,
  active,
}) => (
  <div className='flex-1 min-w-0 flex items-center gap-8px rd-10px bg-fill-2 px-10px py-8px'>
    <span
      className={[
        'flex shrink-0 items-center justify-center w-26px h-26px rd-8px',
        active ? 'text-[rgb(var(--success-6))] bg-[rgba(var(--success-6),0.12)]' : 'text-t-tertiary bg-fill-3',
      ].join(' ')}
    >
      {icon}
    </span>
    <span className='min-w-0 flex flex-col leading-none'>
      <span className='text-16px font-700 text-t-primary'>{count}</span>
      <span className='mt-3px text-11px text-t-tertiary truncate'>{label}</span>
    </span>
  </div>
);

/**
 * 外呼员工卡片 —— 员工「工牌」式卡片：头像叠加盾牌印记、名字、对外服务状态、
 * 绑定渠道 / 公开知识库计数。点击进入配置抽屉。
 */
const EmployeeCard: React.FC<Props> = ({ employee, stats, onOpen }) => {
  const { t } = useTranslation();
  const modelReady = Boolean(employee.model.provider_id && employee.model.model);

  return (
    <div
      onClick={onOpen}
      role='button'
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          onOpen();
        }
      }}
      className='group relative flex flex-col overflow-hidden rd-16px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] cursor-pointer outline-none transition-all hover:border-[rgba(var(--success-6),0.5)] hover:shadow-[0_12px_28px_rgba(var(--success-6),0.14)] hover:-translate-y-2px'
    >
      {/* Header strip — a soft service-green wash behind the identity row. */}
      <div
        className='flex items-start gap-12px px-16px pt-16px pb-12px'
        style={{
          background:
            'linear-gradient(135deg, rgba(var(--success-6),0.10) 0%, rgba(var(--primary-6),0.05) 100%)',
        }}
      >
        <div className='relative shrink-0'>
          <CompanionAvatar
            character={employee.character}
            companionId={employee.id}
            customFigure={customFigureMetaOf(employee)}
            mood={(employee.status.mood as CompanionMood) || 'content'}
            activity='idle'
            size={52}
          />
          {/* Verified "public service" seal. */}
          <span
            className='absolute -right-2px -bottom-2px flex items-center justify-center w-18px h-18px rd-full text-white border-2 border-[var(--color-bg-2)]'
            style={{ background: 'rgb(var(--success-6))' }}
            title={t('outbound.card.verified', { defaultValue: '公开服务客服' })}
          >
            <Shield theme='filled' size='10' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
          </span>
        </div>
        <div className='min-w-0 flex-1 pt-2px'>
          <div className='text-15px font-700 text-t-primary truncate'>{employee.name}</div>
          <div className='mt-5px flex items-center gap-6px flex-wrap'>
            <span className='inline-flex items-center gap-4px rd-full px-8px py-2px text-11px font-600 leading-none text-[rgb(var(--success-6))] bg-[rgba(var(--success-6),0.12)] border border-solid border-[rgba(var(--success-6),0.28)]'>
              <span className='w-6px h-6px rd-full' style={{ background: 'rgb(var(--success-6))' }} />
              {t('outbound.card.serving', { defaultValue: '对外服务中' })}
            </span>
            <span className='inline-flex items-center gap-4px text-11px text-t-tertiary'>
              <span
                className='w-6px h-6px rd-full'
                style={{ background: modelReady ? 'rgb(var(--success-6))' : 'rgb(var(--warning-6))' }}
              />
              {modelReady
                ? t('outbound.card.modelReady', { defaultValue: '模型已配置' })
                : t('outbound.card.modelUnset', { defaultValue: '未配置模型' })}
            </span>
          </div>
        </div>
      </div>

      {/* Stats */}
      <div className='flex items-stretch gap-8px px-16px pt-12px pb-16px'>
        <StatTile
          active={stats.activeChannelCount > 0}
          icon={<Connect theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />}
          count={stats.channelCount}
          label={
            stats.channelCount > 0
              ? t('outbound.card.channelsActive', {
                  defaultValue: '绑定渠道 · {{active}} 在线',
                  active: stats.activeChannelCount,
                })
              : t('outbound.card.channels', { defaultValue: '绑定渠道' })
          }
        />
        <StatTile
          active={stats.kbCount > 0}
          icon={<BookOne theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />}
          count={stats.kbCount}
          label={t('outbound.card.knowledge', { defaultValue: '公开知识库' })}
        />
      </div>
    </div>
  );
};

export default EmployeeCard;
