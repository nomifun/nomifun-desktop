/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Api, Broadcast, Connection } from '@icon-park/react';
import { SectionCard } from '../components';

/**
 * 渠道部署 —— 明确标注的占位区。真实的渠道绑定将在后端集成时接入，
 * 此处仅说明能力，不实现绑定逻辑。
 */
const ChannelsSection: React.FC = () => {
  const { t } = useTranslation();

  const items = [
    {
      icon: <Broadcast theme='outline' size='16' fill='currentColor' className='block' style={{ lineHeight: 0 }} />,
      title: t('publicCompanion.channels.imTitle', { defaultValue: '社交 / IM 渠道' }),
      desc: t('publicCompanion.channels.imDesc', { defaultValue: '把对外伙伴接入 Telegram、飞书等渠道，代表你接待陌生用户。' }),
    },
    {
      icon: <Api theme='outline' size='16' fill='currentColor' className='block' style={{ lineHeight: 0 }} />,
      title: t('publicCompanion.channels.webTitle', { defaultValue: '网页 / API 接入' }),
      desc: t('publicCompanion.channels.webDesc', { defaultValue: '通过网页挂件或 API 把对外伙伴嵌入你的站点与业务系统。' }),
    },
  ];

  return (
    <SectionCard
      icon={<Connection theme='outline' size='16' fill='currentColor' className='block' style={{ lineHeight: 0 }} />}
      title={t('publicCompanion.channels.title', { defaultValue: '渠道部署' })}
      desc={t('publicCompanion.channels.desc', { defaultValue: '把这位对外伙伴部署到真实的对外触点。' })}
      action={
        <span className='inline-flex items-center gap-5px rd-full px-9px py-3px text-11px font-600 leading-none text-[rgb(var(--warning-6))] bg-[rgba(var(--warning-6),0.12)] border border-solid border-[rgba(var(--warning-6),0.26)]'>
          {t('publicCompanion.channels.pending', { defaultValue: '后端集成中' })}
        </span>
      }
    >
      <div
        className='flex flex-col items-center gap-6px rd-12px border border-dashed border-[var(--color-border-2)] bg-fill-1 px-16px py-24px text-center'
      >
        <div className='text-14px font-600 text-t-primary'>
          {t('publicCompanion.channels.placeholderTitle', { defaultValue: '渠道部署（后端集成中）' })}
        </div>
        <div className='text-12px text-t-tertiary leading-18px max-w-[440px]'>
          {t('publicCompanion.channels.placeholderDesc', {
            defaultValue: '渠道绑定将在后端集成完成后开放。届时可在此处把对外伙伴接入社交渠道与网页 / API 触点。',
          })}
        </div>
      </div>

      <div className='mt-14px grid gap-12px' style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(260px, 100%), 1fr))' }}>
        {items.map((it) => (
          <div
            key={it.title}
            className='flex items-start gap-10px rd-12px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-14px py-12px opacity-70'
          >
            <span className='mt-1px flex shrink-0 items-center justify-center w-28px h-28px rd-8px text-t-tertiary bg-fill-2'>
              {it.icon}
            </span>
            <div className='min-w-0'>
              <div className='text-13px font-600 text-t-primary'>{it.title}</div>
              <div className='mt-2px text-12px text-t-tertiary leading-17px'>{it.desc}</div>
            </div>
          </div>
        ))}
      </div>
    </SectionCard>
  );
};

export default ChannelsSection;
