/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { CheckOne, Lock, Shield } from '@icon-park/react';

/** An allowed / blocked capability chip in the safety explainer. */
const CapChip: React.FC<{ label: string; allowed: boolean }> = ({ label, allowed }) => (
  <span
    className={[
      'inline-flex items-center gap-4px rd-full px-9px py-3px text-12px font-500 leading-none border border-solid select-none',
      allowed
        ? 'text-[rgb(var(--success-6))] bg-[rgba(var(--success-6),0.10)] border-[rgba(var(--success-6),0.28)]'
        : 'text-t-tertiary bg-fill-2 border-[var(--color-border-2)]',
    ].join(' ')}
  >
    {allowed ? (
      <CheckOne theme='filled' size='13' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
    ) : (
      <Lock theme='outline' size='12' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
    )}
    <span className={allowed ? '' : 'line-through decoration-1'}>{label}</span>
  </span>
);

/**
 * 外呼员工安全说明横幅 —— 首屏定调：外呼员工 = 面向陌生人的安全客服，
 * 只能问答 + 检索知识库，已关闭电脑控制 / 文件 / 代码 / 浏览器等高危能力。
 * 视觉上以「盾牌 + 绿色可信」母题标记其收窄的安全语义。
 */
const OutboundBanner: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div
      className='relative overflow-hidden rd-16px border border-solid border-[var(--color-border-2)] px-20px py-18px'
      style={{
        background:
          'linear-gradient(120deg, rgba(var(--primary-6),0.07) 0%, rgba(var(--success-6),0.09) 55%, rgba(var(--success-6),0.03) 100%)',
      }}
    >
      {/* Decorative oversized shield glyph bleeding off the right edge. */}
      <Shield
        theme='filled'
        size='150'
        fill='rgba(var(--success-6),0.06)'
        className='pointer-events-none absolute -right-16px -top-24px block'
        style={{ lineHeight: 0 }}
      />
      <div className='relative flex items-start gap-14px'>
        <span
          className='mt-2px flex shrink-0 items-center justify-center w-40px h-40px rd-12px text-white shadow-[0_8px_20px_rgba(var(--success-6),0.28)]'
          style={{ background: 'linear-gradient(160deg, rgb(var(--success-5)), rgb(var(--success-6)))' }}
        >
          <Shield theme='filled' size='22' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
        </span>
        <div className='min-w-0 flex-1'>
          <div className='flex items-center gap-8px'>
            <span className='text-16px font-700 text-t-primary leading-tight'>
              {t('outbound.banner.title', { defaultValue: '外呼员工 · 面向陌生人的安全客服' })}
            </span>
          </div>
          <p className='m-0 mt-4px text-13px leading-20px text-t-secondary max-w-[720px]'>
            {t('outbound.banner.desc', {
              defaultValue:
                '外呼员工是设为「公开服务」的桌面伙伴：只能进行智能问答与知识库检索，已关闭电脑控制、文件读写、代码执行、浏览器等高危能力，可安全地对接社交渠道接待陌生用户。',
            })}
          </p>
          <div className='mt-12px flex flex-wrap items-center gap-x-14px gap-y-8px'>
            <span className='inline-flex items-center gap-6px'>
              <span className='text-11px font-600 text-t-tertiary'>
                {t('outbound.banner.allowedLabel', { defaultValue: '开放' })}
              </span>
              <CapChip allowed label={t('outbound.cap.chat', { defaultValue: '智能问答' })} />
              <CapChip allowed label={t('outbound.cap.knowledge', { defaultValue: '知识库检索' })} />
            </span>
            <span className='inline-flex items-center gap-6px'>
              <span className='text-11px font-600 text-t-tertiary'>
                {t('outbound.banner.blockedLabel', { defaultValue: '已关闭' })}
              </span>
              <CapChip allowed={false} label={t('outbound.cap.computer', { defaultValue: '电脑控制' })} />
              <CapChip allowed={false} label={t('outbound.cap.file', { defaultValue: '文件读写' })} />
              <CapChip allowed={false} label={t('outbound.cap.code', { defaultValue: '代码执行' })} />
              <CapChip allowed={false} label={t('outbound.cap.browser', { defaultValue: '浏览器' })} />
            </span>
          </div>
        </div>
      </div>
    </div>
  );
};

export default OutboundBanner;
