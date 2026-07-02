/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Spin } from '@arco-design/web-react';
import { Plus, Shield } from '@icon-park/react';
import OutboundBanner from './OutboundBanner';
import EmployeeCard from './EmployeeCard';
import HireEmployeeModal from './HireEmployeeModal';
import EmployeeDrawer from './EmployeeDrawer';
import { useOutbound } from './useOutbound';

/**
 * 外呼员工（/outbound）—— 顶级页：管理面向陌生人的安全客服（设为「公开服务」的桌面伙伴）。
 * 列表（卡片网格）+ 招聘（新建专属伙伴并设公开）+ 详情抽屉（知识库/渠道/启停/审计）。
 */
const OutboundEmployeesPage: React.FC = () => {
  const { t } = useTranslation();
  const { employees, loading, statsOf, refreshStats, hireEmployee, setExposure } = useOutbound();

  const [hireOpen, setHireOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const closeDrawer = () => {
    setSelectedId(null);
    // Reconcile roster stats in case a WS binding/channel echo was missed.
    refreshStats();
  };

  return (
    <div className='w-full min-h-full box-border overflow-y-auto px-16px py-20px'>
      <div className='mx-auto flex w-full max-w-[1160px] box-border flex-col gap-16px'>
        {/* Header */}
        <div className='flex items-start justify-between gap-16px flex-wrap'>
          <div className='min-w-0'>
            <h1 className='m-0 mb-4px text-20px font-700 text-t-primary'>
              {t('outbound.title', { defaultValue: '外呼员工' })}
            </h1>
            <p className='m-0 text-13px text-t-secondary'>
              {t('outbound.subtitle', { defaultValue: '面向陌生人的安全客服 —— 只做问答与知识库检索的公开桌面伙伴。' })}
            </p>
          </div>
          <Button type='primary' size='default' className='shrink-0' onClick={() => setHireOpen(true)}>
            <span className='inline-flex items-center gap-6px'>
              <Plus theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
              {t('outbound.hireAction', { defaultValue: '招聘员工' })}
            </span>
          </Button>
        </div>

        <OutboundBanner />

        {/* Roster */}
        {loading ? (
          <div className='flex justify-center py-56px'>
            <Spin />
          </div>
        ) : employees.length === 0 ? (
          <div className='flex flex-col items-center gap-14px rd-16px border border-dashed border-[var(--color-border-2)] bg-fill-1 px-20px py-52px text-center'>
            <span
              className='flex items-center justify-center w-56px h-56px rd-16px text-white shadow-[0_10px_26px_rgba(var(--success-6),0.26)]'
              style={{ background: 'linear-gradient(160deg, rgb(var(--success-5)), rgb(var(--success-6)))' }}
            >
              <Shield theme='filled' size='28' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
            </span>
            <div className='flex flex-col gap-4px'>
              <span className='text-15px font-600 text-t-primary'>
                {t('outbound.empty.title', { defaultValue: '还没有外呼员工' })}
              </span>
              <span className='text-13px text-t-tertiary max-w-[420px]'>
                {t('outbound.empty.desc', {
                  defaultValue: '招聘一位专属的公开客服，绑定知识库与社交渠道，让它安全地接待陌生用户。',
                })}
              </span>
            </div>
            <Button type='primary' onClick={() => setHireOpen(true)}>
              <span className='inline-flex items-center gap-6px'>
                <Plus theme='outline' size='15' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
                {t('outbound.empty.action', { defaultValue: '招聘第一位外呼员工' })}
              </span>
            </Button>
          </div>
        ) : (
          <div
            className='grid gap-16px'
            style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(min(300px, 100%), 1fr))' }}
          >
            {employees.map((e) => (
              <EmployeeCard key={e.id} employee={e} stats={statsOf(e.id)} onOpen={() => setSelectedId(e.id)} />
            ))}
          </div>
        )}
      </div>

      <HireEmployeeModal
        visible={hireOpen}
        onClose={() => setHireOpen(false)}
        onHired={(profile) => setSelectedId(profile.id)}
        hireEmployee={hireEmployee}
      />

      <EmployeeDrawer
        companionId={selectedId}
        onClose={closeDrawer}
        setExposure={setExposure}
        onRetired={() => setSelectedId(null)}
      />
    </div>
  );
};

export default OutboundEmployeesPage;
