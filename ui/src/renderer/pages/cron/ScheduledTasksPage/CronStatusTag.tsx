/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Tag } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import type { ICronJob } from '@/common/adapter/ipcBridge';

type StatusColor = 'gray' | 'red' | 'green' | 'orange';
type StatusTone = 'paused' | 'error' | 'active' | 'input_required' | 'cancelled';

const CronStatusTag: React.FC<{ job: ICronJob }> = ({ job }) => {
  const { t } = useTranslation();

  let color: StatusColor = 'green';
  let label = t('cron.status.active');
  let tone: StatusTone = 'active';

  if (job.state.configuration_error) {
    color = 'orange'; tone = 'input_required'; label = t('cron.status.configurationError');
  } else if (!job.enabled) {
    color = 'gray';
    tone = 'paused';
    label = t('cron.status.paused');
  } else if (job.state.last_status === 'cancelled') {
    color = 'gray'; tone = 'cancelled'; label = t('cron.detail.runStatus.cancelled');
  } else if (job.state.last_status === 'input_required') {
    color = 'orange'; tone = 'input_required'; label = t('cron.detail.runStatus.input_required');
  } else if (job.state.last_status === 'error' || job.state.last_status === 'missed') {
    color = 'red';
    tone = 'error';
    label = t('cron.status.error');
  }

  return (
    <Tag
      size='small'
      bordered
      color={color}
      className={`!rounded-full !px-9px !py-2px !text-12px !leading-16px !font-medium !shadow-none ${
        tone === 'paused' || tone === 'cancelled'
          ? '!bg-fill-1 !border-arco-3 !text-3'
          : tone === 'input_required' ? '!bg-warning-1 !border-warning-4 !text-warning-7'
          : tone === 'error'
            ? '!bg-danger-light-1 !border-danger-4 !text-danger-6'
            : '!bg-success-light-1 !border-success-4 !text-success-6'
      }`}
    >
      {label}
    </Tag>
  );
};

export default CronStatusTag;
