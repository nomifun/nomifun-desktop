/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Tooltip } from '@arco-design/web-react';
import { IconQuestionCircle } from '@arco-design/web-react/icon';

interface LabelWithTipProps {
  label: string;
  tip?: string;
  className?: string;
}

const LabelWithTip: React.FC<LabelWithTipProps> = ({ label, tip, className }) => (
  <label className={`flex items-center gap-4px text-14px font-medium text-t-primary ${className ?? 'mb-6px'}`}>
    {label}
    {tip && (
      <Tooltip content={tip}>
        <IconQuestionCircle className='text-13px text-t-tertiary cursor-help' />
      </Tooltip>
    )}
  </label>
);

export default LabelWithTip;
