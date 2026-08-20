/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import classNames from 'classnames';

interface ModelHubPageHeaderProps {
  title: React.ReactNode;
  description: React.ReactNode;
  badge?: React.ReactNode;
  actions?: React.ReactNode;
  className?: string;
}

/** Shared title treatment for every model-management section. */
const ModelHubPageHeader: React.FC<ModelHubPageHeaderProps> = ({
  title,
  description,
  badge,
  actions,
  className,
}) => (
  <header className={classNames('flex items-start justify-between gap-12px flex-wrap', className)}>
    <div className='min-w-0'>
      <div className='flex min-w-0 items-center gap-8px flex-wrap'>
        <h2 className='m-0 text-15px font-600 leading-20px text-t-primary'>{title}</h2>
        {badge}
      </div>
      <p className='m-0 mt-4px text-12px leading-18px text-t-tertiary'>{description}</p>
    </div>
    {actions}
  </header>
);

export default ModelHubPageHeader;
