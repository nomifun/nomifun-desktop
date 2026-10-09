/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import PageHeader from '@/renderer/components/layout/PageHeader';

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
  <PageHeader level={2} title={title} description={description} badge={badge} actions={actions}
    className={['page-header--flush', className].filter(Boolean).join(' ')} />
);

export default ModelHubPageHeader;
