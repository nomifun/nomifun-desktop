/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import InputNumber from '@arco-design/web-react/es/InputNumber';
import type { InputNumberProps } from '@arco-design/web-react/es/InputNumber';
import type { RefInputType } from '@arco-design/web-react/es/Input/interface';
import classNames from 'classnames';
import React from 'react';

const widthUnitsOf = (value: string): number =>
  Array.from(value).reduce((total, character) => total + ((character.codePointAt(0) ?? 0) > 0xff ? 2 : 1), 0);

export interface NomiInputNumberProps extends InputNumberProps {
  contentFit?: boolean;
  contentMinUnits?: number;
  contentMaxUnits?: number;
}

const NomiInputNumber = React.forwardRef<RefInputType, NomiInputNumberProps>(
  (
    {
      className,
      style,
      value,
      defaultValue,
      placeholder,
      suffix,
      contentFit = false,
      contentMinUnits = 8,
      contentMaxUnits = 20,
      ...rest
    },
    ref
  ) => {
    const valueText = String(value ?? defaultValue ?? placeholder ?? '');
    const suffixText = typeof suffix === 'string' || typeof suffix === 'number' ? String(suffix) : '';
    const contentWidth = `${Math.min(
      contentMaxUnits,
      Math.max(contentMinUnits, widthUnitsOf(valueText) + widthUnitsOf(suffixText) + 5)
    )}ch`;

    return (
      <InputNumber
        ref={ref}
        className={classNames('nomi-input-number', contentFit && 'shrink-0', className)}
        style={{ ...(contentFit ? { width: contentWidth } : undefined), ...style }}
        value={value}
        defaultValue={defaultValue}
        placeholder={placeholder}
        suffix={suffix}
        {...rest}
      />
    );
  }
);

NomiInputNumber.displayName = 'NomiInputNumber';

export default NomiInputNumber;
