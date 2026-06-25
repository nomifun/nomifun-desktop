/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { DatabaseConfig, Excel, Plug } from '@icon-park/react';
import SourceCard from './SourceCard';

/**
 * SourcesPage — the requirements「数据源 / Data Sources」section. A
 * forward-looking display: the local requirements DB is the only real, active
 * source today; 飞书多维表格 and 其他第三方 are shown as "coming soon"
 * placeholders (no backend exists yet — purely presentational).
 *
 * Renders inside RequirementsLayout's content pane, which already provides the
 * scroll container, centered max-width and responsive horizontal padding. So
 * this renders bare section blocks in a plain flex column — matching the
 * WorkspacePage / ExtensionsPage convention (no own scroll / width / padding).
 */
const GRID_STYLE: React.CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fill, minmax(min(260px, 100%), 1fr))',
  gap: '14px',
};

const SourcesPage: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col gap-32px'>
      {/* Section 1 — connected (real, active) */}
      <div className='flex flex-col gap-14px'>
        <div className='text-13px font-600 leading-18px text-t-secondary uppercase tracking-wide'>
          {t('requirements.source.connectedTitle')}
        </div>
        <div style={GRID_STYLE}>
          <SourceCard
            icon={<DatabaseConfig theme='outline' size={20} strokeWidth={3} />}
            name={t('requirements.source.local.name')}
            description={t('requirements.source.local.desc')}
            status='active'
          />
        </div>
      </div>

      {/* Section 2 — upcoming (reserved placeholders) */}
      <div className='flex flex-col gap-14px'>
        <div className='text-13px font-600 leading-18px text-t-secondary uppercase tracking-wide'>
          {t('requirements.source.upcomingTitle')}
        </div>
        <div style={GRID_STYLE}>
          <SourceCard
            icon={<Excel theme='outline' size={20} strokeWidth={3} />}
            name={t('requirements.source.lark.name')}
            description={t('requirements.source.lark.desc')}
            status='soon'
          />
          <SourceCard
            icon={<Plug theme='outline' size={20} strokeWidth={3} />}
            name={t('requirements.source.other.name')}
            description={t('requirements.source.other.desc')}
            status='planned'
          />
        </div>
      </div>

      {/* Muted explanatory note */}
      <p className='m-0 text-12px leading-20px text-t-tertiary'>{t('requirements.source.note')}</p>
    </div>
  );
};

export default SourcesPage;
