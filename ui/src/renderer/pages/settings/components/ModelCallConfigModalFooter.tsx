/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { ModelTask } from '@/common/protocolBindings/ModelTask';
import { Button } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

const ModelCallConfigModalFooter: React.FC<{
  task: ModelTask;
  onCancel: () => void;
  onApply: () => void;
}> = ({ task, onCancel, onApply }) => {
  const { t } = useTranslation();
  const taskLabel = t(`settings.modelTask.${task}`, { defaultValue: task });

  return (
    <div className='flex items-center justify-between gap-10px'>
      <span className='text-11px text-t-secondary'>
        {t('settings.modelAdvanced.pendingDraftScope', {
          defaultValue: '更改只会加入当前模型草稿。',
        })}
      </span>
      <div className='flex gap-8px'>
        <Button onClick={onCancel}>
          {t('settings.modelAdvanced.cancelAdjustment', {
            defaultValue: '取消调整',
          })}
        </Button>
        <Button type='primary' onClick={onApply}>
          {t('settings.modelAdvanced.applyToTask', {
            task: taskLabel,
            defaultValue: '应用到当前任务',
          })}
        </Button>
      </div>
    </div>
  );
};

export default ModelCallConfigModalFooter;
