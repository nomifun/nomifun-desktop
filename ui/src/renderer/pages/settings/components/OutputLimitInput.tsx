/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { InputNumber } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

interface OutputLimitInputProps {
  value?: number;
  onChange?: (value?: number) => void;
}

const normalizeOutputLimit = (value: unknown): number | undefined =>
  typeof value === 'number' && Number.isFinite(value) && value > 0
    ? Math.trunc(value)
    : undefined;

export const OutputLimitInput: React.FC<OutputLimitInputProps> = ({ value, onChange }) => {
  const { t } = useTranslation();

  return (
    <InputNumber
      value={normalizeOutputLimit(value)}
      min={1}
      precision={0}
      step={1}
      style={{ width: '100%' }}
      placeholder={t('settings.outputLimitProviderDefault', {
        defaultValue: 'Leave blank to use the provider default',
      })}
      onChange={(nextValue) => onChange?.(normalizeOutputLimit(nextValue))}
    />
  );
};
